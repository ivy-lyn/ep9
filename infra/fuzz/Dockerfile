# syntax=docker/dockerfile:1
#
# ep9-fuzz image (linux/amd64 only): Fuzzilli at third_party/fuzzilli.commit with the ep9
# profile patch, and `ep9 --reprl` built with SanitizerCoverage (infra/fuzz/build-ep9.sh). The
# Fuzzilli runs on Cloudflare use it (plan v9 §5.5, V9-D24; never on the Mac, rule 23).
#
# The build context is the ep9 checkout root (Dockerfile.dockerignore keeps target/, .git and
# the third_party checkouts out of it):
#   docker build --platform linux/amd64 -f infra/fuzz/Dockerfile -t ep9-fuzz .
#
# As infra/x86-sandbox/Dockerfile: the sandbox-shim stage (the tag must equal the npm
# @cloudflare/sandbox version of the Worker that runs the image) and `sleep infinity` as the
# main process, so a copy of that Worker can run this image.
FROM --platform=linux/amd64 docker.io/cloudflare/sandbox:1.0.1 AS sandbox-shim

# Swift 6.3.3 on Debian 12 (Fuzzilli's Package.swift needs swift-tools-version 6.3).
# Pinned by multi-arch index digest (2026-10-27); --platform selects the amd64 manifest.
FROM --platform=linux/amd64 docker.io/library/swift:6.3.3-bookworm@sha256:833eff2d67810d6bd596be66de164af9eca834fabee9b8341a3ffafcee9339e6

COPY --from=sandbox-shim /usr/local/bin/sandbox-shim /usr/local/bin/sandbox-shim

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config git curl ca-certificates zstd jq python3 procps gdb \
 && rm -rf /var/lib/apt/lists/*

# Rust: rustup stable (minimal); the build below installs the rust-toolchain.toml pin.
ENV PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
 && rm -rf /root/.rustup/downloads /root/.rustup/tmp

COPY . /src
WORKDIR /src

# Fuzzilli (pin + patch) -> /opt/fuzzilli/bin; ep9 (sancov) -> /opt/ep9/bin/ep9. The build
# trees are removed; the sources stay for symbolized backtraces.
RUN bash scripts/sync-fuzzilli.sh \
 && bin="$(cd third_party/fuzzilli && swift build -c release --show-bin-path)" \
 && install -D -m 755 "$bin/FuzzilliCli" /opt/fuzzilli/bin/FuzzilliCli \
 && install -D -m 755 "$bin/REPRLRun" /opt/fuzzilli/bin/REPRLRun \
 && rm -rf third_party/fuzzilli/.build \
 && rustup toolchain install \
 && ep9="$(bash infra/fuzz/build-ep9.sh x86_64-unknown-linux-gnu)" \
 && install -D -m 755 "$ep9" /opt/ep9/bin/ep9 \
 && rm -rf target /root/.cargo/registry \
 && /opt/fuzzilli/bin/FuzzilliCli --help | grep -q '"ep9"'

ENV EP9=/opt/ep9/bin/ep9 \
    FUZZILLI=/opt/fuzzilli/bin/FuzzilliCli

RUN mkdir -p /work
WORKDIR /work
ENTRYPOINT []
CMD ["sleep", "infinity"]
