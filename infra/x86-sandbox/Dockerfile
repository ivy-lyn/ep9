# syntax=docker/dockerfile:1
#
# ep9-x86-sandbox image (linux/amd64 only). Run, test, bench and debug ep9
# x86-64 binaries that were cross-built on the Mac with cargo-zigbuild.
#
# @cloudflare/sandbox 1.0.x ships no base image. The `cloudflare/sandbox:<VERSION>`
# image only carries the `sandbox-shim` helper, and its tag must match the npm
# package version (1.0.1, pinned in package.json); a mismatch makes `Files`
# throw SandboxProtocolError.
FROM --platform=linux/amd64 docker.io/cloudflare/sandbox:1.0.1 AS sandbox-shim

# Debian 13 (trixie) + Node.js 24 (ep9 tests compare their output with node).
# Pinned by multi-arch index digest; --platform selects the amd64 manifest.
FROM --platform=linux/amd64 docker.io/library/node:24-trixie-slim@sha256:7f486cabe892bff8fef04c0f7da8dba84a560be3e4e2740b7a86afc57ebec315

COPY --from=sandbox-shim /usr/local/bin/sandbox-shim /usr/local/bin/sandbox-shim

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

# Debug / bench tools: valgrind (cachegrind for ep9-bench), perf, gdb, strace.
# build-essential gives rustup a linker for small on-box rebuilds.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config \
      valgrind linux-perf gdb strace \
      git curl ca-certificates xz-utils zstd file \
      python3 time jq procps util-linux \
 && rm -rf /var/lib/apt/lists/*

# Rust (stable, minimal profile) for root, at rustup's default location
# (/root/.rustup, /root/.cargo). exec() only inherits PATH from the image, so
# the Worker passes HOME=/root and puts /root/.cargo/bin on PATH.
ENV PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
 && rustc --version && cargo --version \
 && rm -rf /root/.rustup/downloads /root/.rustup/tmp

RUN mkdir -p /work/jobs /work/.upload

WORKDIR /work
# The node image sets ENTRYPOINT docker-entrypoint.sh; clear it so the main
# process is just `sleep infinity` (keeps the instance up for exec()).
ENTRYPOINT []
CMD ["sleep", "infinity"]
