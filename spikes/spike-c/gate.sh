#!/usr/bin/env bash
# Spike C gate (M1; docs/measurements/2026-10-27-spike-c-protocol.md). Exit 0 only if every step
# that runs passes. Each later SPIKEC role adds its step here (plan v9: SPIKEC(step)).
#
#   gate.sh structural   (plan v9 M4-P0 P0-SPIKEC task 3; the SPIKEC(structural) acceptance)
#     1  cargo test: the spike's unit tests (harness, facts round trip, kernel drivers and hot
#        lines, the (b) install path, the pipeline stubs)
#     2  release build (panic = "abort"): `spike-c structural all`, the same checks in the
#        release build, where T0 runs the threaded dispatch and placement (b) code really runs.
#        The kernels check prints SKIP without the v8-v7 checkout (scripts/sync-bench.sh).
#   gate.sh              `gate.sh structural`
#
# Env: CARGO_TARGET_DIR (default: spikes/spike-c/target),
#      SPIKE_C_BENCH_DIR (default: third_party/bench-v8v7/bench/bench-v8).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"
host="$(rustc -vV | sed -n 's/^host: //p')"
exe=""
case "$host" in *windows*) exe=".exe" ;; esac

results=()
failed=0
record() { # id status note
  results+=("$(printf '%-24s %-5s %s' "$1" "$2" "$3")")
  if [ "$2" = FAIL ]; then failed=1; fi
}
run() { # id cmd...: runs cmd, records PASS or FAIL
  local id="$1"
  shift
  echo "== [$id] $*"
  if "$@"; then record "$id" PASS ""; else record "$id" FAIL "exit $?"; fi
}

structural() {
  run "unit-tests" cargo test --locked
  run "build-release" cargo build --locked --release
  if [ -x "$CARGO_TARGET_DIR/release/spike-c$exe" ]; then
    "$CARGO_TARGET_DIR/release/spike-c$exe"
    run "structural/release" "$CARGO_TARGET_DIR/release/spike-c$exe" structural all
  else
    record "structural/release" FAIL "no release binary"
  fi
}

step="${1:-all}"
case "$step" in
  structural | all) structural ;;
  *)
    echo "usage: gate.sh [structural]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike C gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEC($step): PASS"; else echo "SPIKEC($step): FAIL"; fi
exit "$failed"
