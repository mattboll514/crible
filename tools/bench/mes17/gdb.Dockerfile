# Annex image of the MES-17 third campaign (ADR-0159, decision 2).
#
# It carries `gdb` and nothing else, and it is the only place where `gdb` lives:
# the image is neither a build nor a runtime dependency of Crible, it never
# enters `docker/Dockerfile` nor `docs/07`, and it is never used by the server.
# Its role is to run `gdb -p 1 -batch -ex 'call (int) malloc_trim(0)'` in the PID
# namespace of a throwaway measurement container (`--pid=container:<name>`,
# `--cap-add SYS_PTRACE`), then to disappear (`--rm`).
#
# The base is pinned by digest, and it is the same Debian release as the
# `runtime` stage of `docker/Dockerfile` (`ARG DEBIAN_VERSION=bookworm`), so the
# glibc of the debugging session matches the glibc of the measured process.
FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc

RUN apt-get update \
    && apt-get install -y --no-install-recommends gdb \
    && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["gdb"]
