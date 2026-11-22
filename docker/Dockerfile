# syntax=docker/dockerfile:1
#
# Server image of Crible (docs/07-stack-et-deploiement.md, section "Docker").
#
# One CPU image, multi-architecture: `linux/arm64` for the two Macs and
# `linux/amd64` for the future target (ADR-0129). The build follows the model of
# NextPlaid: a `cargo-chef` stage caches the dependencies, a release build
# produces `crible`, and the runtime stage is `debian:bookworm-slim` with the
# ONNX Runtime CPU library of the target architecture.
#
# Build and run: see `docker/README.md`.

# Toolchain of `rust-toolchain.toml`, which is the source of truth: a bump of
# its `channel` must update this default too.
ARG RUST_VERSION=1.99.1
ARG DEBIAN_VERSION=bookworm
ARG CARGO_CHEF_VERSION=0.1.78
# ONNX Runtime 1.31.0, the version of the `ci` group of docs/07: the library
# that `ORT_DYLIB_PATH` names must stay aligned with the `onnxruntime` wheel the
# tests use. The library comes from the pinned PyPI wheel (ADR-0161), not from
# the release archives of `microsoft/onnxruntime`; the URL and the SHA-256 below
# are the ones of `tools/uv.lock` for `onnxruntime-1.31.0` `cp311`
# `manylinux_2_28`, one per target architecture. The wheel names, the sums and
# `tools/uv.lock` move together (ADR-0161, decision 3): the image and the CI
# load the same pinned file.
ARG ORT_VERSION=1.31.0
ARG ORT_WHEEL_URL_AMD64=https://files.pythonhosted.org/packages/a6/72/6c57163b63b5343853d7f0619c4f424a6e53ee762d7263667ff004bfede1/onnxruntime-1.31.0-cp311-cp311-manylinux_2_28_x86_64.whl
ARG ORT_WHEEL_SHA256_AMD64=d4092b78fc5bab77ce6522393098cdb2535423045ecdcff15cc0d022162d6b66
ARG ORT_WHEEL_URL_ARM64=https://files.pythonhosted.org/packages/44/86/e57025ab9c1eb83b6e686c92507fa6b7156d9d375e197a6c3a2afc05a1e2/onnxruntime-1.31.0-cp311-cp311-manylinux_2_28_aarch64.whl
ARG ORT_WHEEL_SHA256_ARM64=37c7dfe398550afdf9670a29315dbb88e49d8afc473ffaf1f410376efbb9c80a

# =============================================================================
# Chef stage: Rust toolchain, build dependencies and `cargo-chef`.
# =============================================================================
FROM rust:${RUST_VERSION}-${DEBIAN_VERSION} AS chef

# `build-essential` and `pkg-config` cover the C parts of the stack (`rusqlite`
# bundled, `zstd`); `cmake` is available for crates that probe it.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

ARG CARGO_CHEF_VERSION
RUN cargo install cargo-chef --version "${CARGO_CHEF_VERSION}" --locked

WORKDIR /build

# =============================================================================
# Planner stage: the dependency recipe.
# =============================================================================
FROM chef AS planner

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# Builder stage: dependencies first (cached by the recipe), then `crible`.
# =============================================================================
FROM chef AS builder

COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo build --release --locked -p crible

# =============================================================================
# ONNX Runtime stage: the library of the pinned wheel (ADR-0161).
#
# A wheel is a ZIP archive, so `unzip` is needed here and only here: the runtime
# stage receives the library by `COPY --from` and no extraction tool. The wheel
# is checked against the SHA-256 of `tools/uv.lock` before anything is
# extracted; a different archive fails the build.
# =============================================================================
FROM debian:${DEBIAN_VERSION}-slim AS onnxruntime

ARG TARGETARCH
ARG ORT_VERSION
ARG ORT_WHEEL_URL_AMD64
ARG ORT_WHEEL_SHA256_AMD64
ARG ORT_WHEEL_URL_ARM64
ARG ORT_WHEEL_SHA256_ARM64

RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) ort_url="${ORT_WHEEL_URL_AMD64}"; ort_sha="${ORT_WHEEL_SHA256_AMD64}" ;; \
        arm64) ort_url="${ORT_WHEEL_URL_ARM64}"; ort_sha="${ORT_WHEEL_SHA256_ARM64}" ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl unzip; \
    rm -rf /var/lib/apt/lists/*; \
    ort_whl=/tmp/onnxruntime.whl; \
    curl -fsSL -o "${ort_whl}" "${ort_url}"; \
    echo "${ort_sha}  ${ort_whl}" | sha256sum -c -; \
    mkdir -p /opt/ort; \
    unzip -j "${ort_whl}" \
        "onnxruntime/capi/libonnxruntime.so.${ORT_VERSION}" \
        -d /opt/ort; \
    rm -f "${ort_whl}"

# =============================================================================
# Runtime stage: `debian:bookworm-slim`, non-root user, ONNX Runtime CPU.
# =============================================================================
FROM debian:${DEBIAN_VERSION}-slim AS runtime

ARG ORT_VERSION

# `ca-certificates` ships the trust store; `curl` serves the health check.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# ONNX Runtime CPU library of the target architecture, extracted from the
# pinned PyPI wheel in the `onnxruntime` stage above (ADR-0161). Only
# `libonnxruntime.so.1.31.0` is copied, and only that file is named by
# `ORT_DYLIB_PATH`: the `capi/` directory of the wheel carries other files, and
# the ones the library does not need stay out of the image.
COPY --from=onnxruntime /opt/ort/libonnxruntime.so.${ORT_VERSION} /opt/ort/

# ONNX Runtime is loaded dynamically (`ort` with `load-dynamic`): the library is
# named by `ORT_DYLIB_PATH`, never linked at build time.
ENV ORT_DYLIB_PATH=/opt/ort/libonnxruntime.so.${ORT_VERSION}
ENV LD_LIBRARY_PATH=/opt/ort
# ADR-0128, decision 6: the 1DS telemetry client of ONNX Runtime stays off, even
# if a binary other than `crible` loads the library in the container.
ENV ORT_DISABLE_TELEMETRY=1

RUN useradd --create-home --uid 1000 --shell /usr/sbin/nologin crible \
    && mkdir -p /data /models /exchange /etc/crible \
    && chown -R crible:crible /data /models /exchange

COPY --from=builder /build/target/release/crible /usr/local/bin/crible

# `/data` and `/models` are named volumes, `/exchange` a directory mounted from
# the host, `/etc/crible/crible.toml` the configuration, read-only
# (docs/07-stack-et-deploiement.md, section "Docker").
USER crible

EXPOSE 8080

# `/health` answers `200` from the bind, whatever the loader is doing (ADR-0099);
# `/ready` waits for the models and the indexes, which is a start condition, not
# a liveness one.
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS --max-time 4 http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["crible"]
CMD ["serve"]
