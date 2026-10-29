#!/usr/bin/env bash
# Builds the ep9 that Fuzzilli runs: `ep9 --reprl` with SanitizerCoverage edge guards
# (trace-pc-guard; the callbacks are in crates/ep9_cli/src/reprl.rs), on the stable toolchain of
# rust-toolchain.toml (probe P-SANCOV: `-Cpasses=sancov-module` and `-Cllvm-args` are stable).
#
#   infra/fuzz/build-ep9.sh [--jitless] [<target triple>]     default: the host triple
#
# Two builds (plan v10 §3.6, MC-w15-13): the default one has ep9_cli's default features (`jit`;
# `ep9 --version` line 2 "features jit; preset jit") and goes to target/sancov; --jitless builds
# `-p ep9_cli --no-default-features` (no `jit` feature, H27; "features none; preset jitless")
# and goes to target/sancov-jitless, so the two never share a target dir or a binary path.
#
# The profile is release with debug assertions and overflow checks on (the fuzz build should
# catch what a release build lets pass, Fuzzilli Targets/README.md), thin local LTO off and 16
# codegen units (faster to build; coverage needs no LTO). Every ep9 crate and every crates.io
# dependency is instrumented; std is not (it is not rebuilt). An explicit --target keeps
# RUSTFLAGS off the host artifacts, so build scripts and proc macros (oxc) are not instrumented
# (an instrumented proc macro would need the callbacks inside rustc).
#
# Output dir: target/sancov or target/sancov-jitless (their own, so the instrumented build never
# mixes with the normal one).
# Environment: CARGO_BUILD_CMD (default "build"; "zigbuild" for a cross build with
# cargo-zigbuild, as the x86-sandbox README does). Prints the binary path on the last line.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
features=() out="$root/target/sancov"
if [ "${1:-}" = --jitless ]; then
  features=(--no-default-features)
  out="$root/target/sancov-jitless"
  shift
fi
case "${1:-}" in -*) echo "usage: infra/fuzz/build-ep9.sh [--jitless] [<target triple>]" >&2; exit 2 ;; esac
triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
[ -n "$triple" ] || { echo "build-ep9: cannot read the host triple from rustc -vV" >&2; exit 2; }
export RUSTFLAGS="${RUSTFLAGS:-} -Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=3 -Cllvm-args=-sanitizer-coverage-trace-pc-guard"
export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true
export CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true
export CARGO_PROFILE_RELEASE_LTO=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
cargo "${CARGO_BUILD_CMD:-build}" --release --locked -p ep9_cli --bin ep9 ${features[@]+"${features[@]}"} \
  --target "$triple" --target-dir "$out" >&2
echo "$out/$triple/release/ep9"
