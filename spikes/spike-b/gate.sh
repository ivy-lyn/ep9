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
#     4  bench-verdict-test.sh: the verdict logic of bench.sh on fake inputs (no bench run);
#        SKIP when node is not on PATH
#   gate.sh b            (plan-v6 P4-PLACEMENT-B task 9; the SPIKEB(b) acceptance)
#     1  cargo test, the jit_b tests: the (b) emitter (prologue, no `ret`, no address, CFI)
#     2  release build: `spike-b structural-b all`, the opcode lists and checks (a)-(h) on
#        placement (b) code (src/jit_b/structural.rs), T0 under Dispatch::Sibling
#     3  debug build: `spike-b structural-b h` (debug sibling calls have no tail calls, so
#        the long checks run in release only)
#     4  bench-verdict-test.sh (the B cases too), if `structural` did not run it already
#     On a target where placement (b) is not built (x86-64), steps 2-3 print SKIP with the
#     `Refusal` text and pass.
#   gate.sh              `gate.sh structural`, then `gate.sh b`
#
# Env: CARGO_TARGET_DIR (default: spikes/spike-b/target),
#      SPIKE_B_DEBUG_ALL=1 runs all of (a)-(h) in step 3 of `structural`, not only (h).
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
  # The test drives bench.sh's host check, which needs node. The gate host and the CI runners
  # have node; a bare container (local CI linux arm64 Docker) does not, and the logic it tests
  # is the same shell on every host.
  verdict
}

verdict_ran=0
verdict() {
  if [ "$verdict_ran" = 1 ]; then return; fi
  verdict_ran=1
  if command -v node >/dev/null; then
    run "bench-verdict" bash "$here/bench-verdict-test.sh"
  else
    record "bench-verdict" SKIP "no node on PATH"
  fi
}

placement_b() {
  run "b/unit-tests" cargo test --locked jit_b
  run "b/build-release" cargo build --locked --release
  if [ -x "$CARGO_TARGET_DIR/release/spike-b$exe" ]; then
    run "structural-b/release" "$CARGO_TARGET_DIR/release/spike-b$exe" structural-b all
  else
    record "structural-b/release" FAIL "no release binary"
  fi
  run "b/build-debug" cargo build --locked
  if [ -x "$CARGO_TARGET_DIR/debug/spike-b$exe" ]; then
    run "structural-b/debug-h" "$CARGO_TARGET_DIR/debug/spike-b$exe" structural-b h
  else
    record "structural-b/debug-h" FAIL "no debug binary"
  fi
  verdict
}

step="${1:-all}"
case "$step" in
  structural) structural ;;
  b) placement_b ;;
  all)
    structural
    placement_b
    ;;
  *)
    echo "usage: gate.sh [structural | b]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike B gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEB($step): PASS"; else echo "SPIKEB($step): FAIL"; fi
exit "$failed"
