#!/usr/bin/env bash
# Spike B gate (M0 month 2; docs/measurements/2026-10-22-spike-b-protocol.md, H14).
# Exit 0 only if every step that runs passes.
#
#   gate.sh structural   (plan-v4 P2-SPIKE-B task 4; the SPIKEB(structural) acceptance)
#     1  cargo test: the spike's unit tests (plan, both generators, refusals)
#     2  release build (panic = "abort"): `spike-b structural all`, which prints the opcode
#        list of each JITed function, then runs checks (a)-(h)
#     3  debug build (panic = "unwind"): `spike-b structural h`, the abort at the
#        non-unwinding table-function boundary (M2-P1-NE-PANIC)
#     4  bench-verdict-test.sh: the verdict logic of bench.sh on fake inputs (no bench run)
#   gate.sh              the same as `gate.sh structural` (later phases add more steps)
#
# Env: CARGO_TARGET_DIR (default: spikes/spike-b/target),
#      SPIKE_B_DEBUG_ALL=1 runs all of (a)-(h) in step 3, not only (h).
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
  results+=("$(printf '%-26s %-5s %s' "$1" "$2" "$3")")
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
  if [ -x "$CARGO_TARGET_DIR/release/spike-b$exe" ]; then
    run "structural/release" "$CARGO_TARGET_DIR/release/spike-b$exe" structural all
  else
    record "structural/release" FAIL "no release binary"
  fi
  run "build-debug" cargo build --locked
  local which=h
  if [ "${SPIKE_B_DEBUG_ALL:-0}" = 1 ]; then which=all; fi
  if [ -x "$CARGO_TARGET_DIR/debug/spike-b$exe" ]; then
    run "structural/debug-$which" "$CARGO_TARGET_DIR/debug/spike-b$exe" structural "$which"
  else
    record "structural/debug-$which" FAIL "no debug binary"
  fi
  run "bench-verdict" bash "$here/bench-verdict-test.sh"
}

step="${1:-structural}"
case "$step" in
  structural) structural ;;
  *)
    echo "usage: gate.sh [structural]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike B gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEB($step): PASS"; else echo "SPIKEB($step): FAIL"; fi
exit "$failed"
