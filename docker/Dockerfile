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

ARG RUST_VERSION=1.99.1
ARG DEBIAN_VERSION=bookworm
ARG CARGO_CHEF_VERSION=0.1.78
# ONNX Runtime 1.31.0, the version of the `ci` group of docs/07: the library
# that `ORT_DYLIB_PATH` names must stay aligned with the `onnxruntime` wheel the
# tests use. Both tarballs are the official Linux CPU releases of the
# `microsoft/onnxruntime` repository, pinned by SHA-256 below.
ARG ORT_VERSION=1.31.0
ARG ORT_SHA256_AMD64=cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57
ARG ORT_SHA256_ARM64=c5b8b3cca31f3d643a3b313b8f42e2d0f16a4fcefd02cda343d97bd5306afaab

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
# Runtime stage: `debian:bookworm-slim`, non-root user, ONNX Runtime CPU.
# =============================================================================
FROM debian:${DEBIAN_VERSION}-slim AS runtime

ARG TARGETARCH
ARG ORT_VERSION
ARG ORT_SHA256_AMD64
ARG ORT_SHA256_ARM64

# `ca-certificates` ships the trust store; `curl` serves the health check.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# ONNX Runtime CPU library of the target architecture, from the official
# release, checked against the pinned SHA-256. Only `libonnxruntime.so*` is
# kept: the headers, the CMake files and the providers library are build-time
# artefacts the image does not need.
RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) ort_arch=x64; ort_sha="${ORT_SHA256_AMD64}" ;; \
        arm64) ort_arch=aarch64; ort_sha="${ORT_SHA256_ARM64}" ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    ort_tgz="/tmp/onnxruntime-linux-${ort_arch}-${ORT_VERSION}.tgz"; \
    curl -fsSL -o "${ort_tgz}" \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ort_arch}-${ORT_VERSION}.tgz"; \
    echo "${ort_sha}  ${ort_tgz}" | sha256sum -c -; \
    mkdir -p /opt/ort; \
    tar -xzf "${ort_tgz}" -C /tmp; \
    cp -a "/tmp/onnxruntime-linux-${ort_arch}-${ORT_VERSION}/lib/libonnxruntime.so"* /opt/ort/; \
    chown -R root:root /opt/ort; \
    rm -rf "${ort_tgz}" "/tmp/onnxruntime-linux-${ort_arch}-${ORT_VERSION}"

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
