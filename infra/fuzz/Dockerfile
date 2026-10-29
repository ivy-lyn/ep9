# syntax=docker/dockerfile:1
#
# ep9-fuzz image (linux/amd64 only): Fuzzilli at third_party/fuzzilli.commit with the ep9
# profile patch, and two `ep9 --reprl` builds with SanitizerCoverage (infra/fuzz/build-ep9.sh):
# /opt/ep9/bin/ep9 (default features, `jit`; preset jit) and /opt/ep9/bin/ep9-jitless
# (`--no-default-features`; preset jitless). The Fuzzilli runs on Cloudflare use it (plan v9
# §5.5, V9-D24; never on the Mac, rule 23). It also holds the cargo-fuzz triage tools (the fuzz
# nightly of fuzz/rust-toolchain.toml and cargo-fuzz 0.12.0, plan v10 V10-D26): a crash that
# a GitHub cargo-fuzz run found is reproduced on a box from this image (fuzz/README.md), so its
# input never leaves the box.
#
#   stage          what                                          runs in parallel with
#   sandbox-shim   the Cloudflare sandbox shim
#   base           apt tools, rustup stable                      -
#   fuzzilli       Fuzzilli (pin + patch) -> /opt/fuzzilli/bin    cargo-fuzz, the ep9 builds
#   cargo-fuzz     cargo install cargo-fuzz 0.12.0                fuzzilli, the ep9 builds
#   (last)         the ep9 builds, both tools, the fuzz nightly   -
#
# The fuzzilli stage copies only its own inputs, so an ep9 source change does not rebuild
# Fuzzilli (a cached layer).
#
# The build context is the ep9 checkout root (Dockerfile.dockerignore keeps target/, .git and
# the third_party checkouts out of it). The two build args name the source state of the
# context; infra/fuzz/run.sh copies them into each ep9-fuzz/v1 record (plan v10 §2.7, CX6-1):
#   docker build --platform linux/amd64 -f infra/fuzz/Dockerfile -t ep9-fuzz \
#     --build-arg EP9_SOURCE_COMMIT="$(git rev-parse HEAD)" \
#     --build-arg EP9_SOURCE_DIRTY="$(test -z "$(git status --porcelain --untracked-files=no)" && echo false || echo true)" .
# For a `git archive` export of HEAD the context is clean: EP9_SOURCE_DIRTY=false. A missing
# or malformed value is written as unknown: the record then has commit "unknown" or no `dirty`
# field, and m1-exit does not count it.
#
# As infra/x86-sandbox/Dockerfile: the sandbox-shim stage (the tag must equal the npm
# @cloudflare/sandbox version of the Worker that runs the image) and `sleep infinity` as the
# main process, so a copy of that Worker can run this image.
FROM --platform=linux/amd64 docker.io/cloudflare/sandbox:1.0.1 AS sandbox-shim

# Swift 6.3.3 on Debian 12 (Fuzzilli's Package.swift needs swift-tools-version 6.3).
# Pinned by multi-arch index digest (2026-10-27); --platform selects the amd64 manifest.
FROM --platform=linux/amd64 docker.io/library/swift:6.3.3-bookworm@sha256:833eff2d67810d6bd596be66de164af9eca834fabee9b8341a3ffafcee9339e6 AS base

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

# Fuzzilli (pin + patch) -> /opt/fuzzilli/bin. The build tree is removed; the sources stay
# (copied to /src/third_party/fuzzilli below) for symbolized backtraces.
FROM base AS fuzzilli
WORKDIR /fz
COPY scripts/sync-fuzzilli.sh scripts/sync-lib.sh scripts/
COPY third_party/fuzzilli.commit third_party/
COPY third_party/patches/fuzzilli third_party/patches/fuzzilli
RUN bash scripts/sync-fuzzilli.sh \
 && bin="$(cd third_party/fuzzilli && swift build -c release --show-bin-path)" \
 && install -D -m 755 "$bin/FuzzilliCli" /opt/fuzzilli/bin/FuzzilliCli \
 && install -D -m 755 "$bin/REPRLRun" /opt/fuzzilli/bin/REPRLRun \
 && rm -rf third_party/fuzzilli/.build

# cargo-fuzz for the triage path (the version of CI's fuzz-smoke job and fuzz/README.md).
FROM base AS cargo-fuzz
RUN cargo install cargo-fuzz --version 0.12.0 --locked --root /opt/cargo-fuzz \
 && rm -rf /root/.cargo/registry

FROM base
COPY . /src
WORKDIR /src

# ep9, two sancov builds: /opt/ep9/bin/ep9 (jit) and /opt/ep9/bin/ep9-jitless. The regress
# checkout (third_party/regress, git-ignored and not in the context) is synced before the ep9
# build: the root [patch.crates-io] path needs it (plan v10 rule 42). The second line of
# `ep9 --version` names the build (features, default preset): each binary must be the one its
# name says.
RUN rustup toolchain install \
 && bash scripts/sync-regress.sh \
 && ep9="$(bash infra/fuzz/build-ep9.sh x86_64-unknown-linux-gnu)" \
 && install -D -m 755 "$ep9" /opt/ep9/bin/ep9 \
 && ep9="$(bash infra/fuzz/build-ep9.sh --jitless x86_64-unknown-linux-gnu)" \
 && install -D -m 755 "$ep9" /opt/ep9/bin/ep9-jitless \
 && rm -rf target /root/.cargo/registry \
 && /opt/ep9/bin/ep9 --version | sed -n 2p | grep -Eq '^features jit(,[a-z-]+)*; preset jit$' \
 && /opt/ep9/bin/ep9-jitless --version | sed -n 2p | grep -qx 'features none; preset jitless'

# The cargo-fuzz triage tools: the fuzz nightly (fuzz/rust-toolchain.toml) and cargo-fuzz.
# The fuzz targets are built on the box when a triage needs them (fuzz/README.md).
COPY --from=cargo-fuzz /opt/cargo-fuzz/bin/cargo-fuzz /root/.cargo/bin/cargo-fuzz
RUN chan="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' fuzz/rust-toolchain.toml)" \
 && rustup toolchain install "$chan" --profile minimal --no-self-update \
 && rm -rf /root/.rustup/downloads /root/.rustup/tmp \
 && (cd fuzz && cargo fuzz --version)

COPY --from=fuzzilli /opt/fuzzilli/bin /opt/fuzzilli/bin
COPY --from=fuzzilli /fz/third_party/fuzzilli /src/third_party/fuzzilli
RUN /opt/fuzzilli/bin/FuzzilliCli --help | grep -q '"ep9"'

# /opt/ep9/SOURCE (read by infra/fuzz/run.sh): "commit=<40 hex | unknown>" and
# "dirty=<true | false | unknown>". After the build, so the build layer does not depend on it.
ARG EP9_SOURCE_COMMIT=
ARG EP9_SOURCE_DIRTY=
RUN c="$EP9_SOURCE_COMMIT"; d="$EP9_SOURCE_DIRTY"; \
    printf '%s' "$c" | grep -Eqx '[0-9a-f]{40}' || c=unknown; \
    case "$d" in true|false) ;; *) d=unknown ;; esac; \
    if [ "$c" = unknown ]; then d=unknown; fi; \
    printf 'commit=%s\ndirty=%s\n' "$c" "$d" > /opt/ep9/SOURCE \
 && cat /opt/ep9/SOURCE

ENV EP9=/opt/ep9/bin/ep9 \
    EP9_JITLESS=/opt/ep9/bin/ep9-jitless \
    FUZZILLI=/opt/fuzzilli/bin/FuzzilliCli

RUN mkdir -p /work
WORKDIR /work
ENTRYPOINT []
CMD ["sleep", "infinity"]
