#!/usr/bin/env bash
# Spike B gate (M0 month 2; docs/measurements/2026-10-22-spike-b-protocol.md, H14).
# Exit 0 only if every step that runs passes.
#
#   gate.sh structural   (plan-v4 P2-SPIKE-B task 4; the SPIKEB(structural) acceptance)
#     1  cargo test: the spike's unit tests (plan, both generators, refusals) and, on
#        Windows, tests/win64_unwind.rs
#     2  release build (panic = "abort"): `spike-b structural all`, which prints the opcode
#        list of each JITed function, then runs checks (a)-(h)
#     3  debug build (panic = "unwind"): `spike-b structural h`, the abort at the
#        non-unwinding table-function boundary (M2-P1-NE-PANIC)
#     4  bench-verdict-test.sh: the verdict logic of bench.sh on fake inputs (no bench run);
#        SKIP when node is not on PATH
#   gate.sh b            (plan-v6 P4-PLACEMENT-B task 9; the SPIKEB(b) acceptance)
#     1  cargo test, the jit_b tests: the (b) emitters of both architectures (prologue, plan,
#        exit jumps, no `ret`, no address, CFI, Win64 UNWIND_INFO)
#     2  release build: `spike-b structural-b all`, the opcode lists and checks (a)-(h) on
#        placement (b) code (src/jit_b/structural.rs), T0 under Dispatch::Sibling
#     3  debug build: `spike-b structural-b h` (debug sibling calls have no tail calls, so
#        the long checks run in release only)
#     4  bench-verdict-test.sh (the B cases too), if `structural` did not run it already
#     Placement (b) is built for AArch64 and x86-64 (M3-P0 P0-B-X64). On an aarch64 or x86_64
#     host a SKIP line in the output of steps 2-3 is a FAIL (P0-D9): it would hide a lost
#     generator. On any other host steps 2-3 print SKIP with the `Refusal` text and pass.
#   gate.sh unit         (plan-v8 M3-P1 SP-UNIT; the SPIKEB(unit) acceptance, M0 exit item 11)
#     1  cargo test --test unit: the checks of `spike-b unit` at test size (debug), one test
#        each; cargo test --lib unit: the relocation scan and the unit code generators
#     2  release build: `spike-b unit` (no argument: every check (a)-(g) at gate size): unit
#        text with 0 relocations, load, RW copies through x27 / r14, 2 isolates on 2 threads,
#        unwind, every fallback case (src/unit/checks.rs)
#     A SKIP line in step 2 fails on an aarch64 or x86_64 host, as in `gate.sh b`.
#   gate.sh              `gate.sh structural`, then `gate.sh b`, then `gate.sh unit`
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

# The hosts that have a placement (b) generator: a SKIP from structural-b fails there (P0-D9).
case "$(uname -m)" in
  arm64 | aarch64 | x86_64 | amd64) b_host=1 ;;
  *) b_host=0 ;;
esac

run_b() { # id cmd...: as `run`, plus FAIL on a SKIP line where (b) is built
  local id="$1"
  shift
  echo "== [$id] $*"
  local log
  log="$(mktemp)"
  if "$@" | tee "$log"; then
    if [ "$b_host" = 1 ] && grep -q SKIP "$log"; then
      record "$id" FAIL "a SKIP line on $(uname -m), where placement (b) is built"
    else
      record "$id" PASS ""
    fi
  else
    record "$id" FAIL "exit $?"
  fi
  rm -f "$log"
}

placement_b() {
  run "b/unit-tests" cargo test --locked jit_b
  run "b/build-release" cargo build --locked --release
  if [ -x "$CARGO_TARGET_DIR/release/spike-b$exe" ]; then
    run_b "structural-b/release" "$CARGO_TARGET_DIR/release/spike-b$exe" structural-b all
  else
    record "structural-b/release" FAIL "no release binary"
  fi
  run "b/build-debug" cargo build --locked
  if [ -x "$CARGO_TARGET_DIR/debug/spike-b$exe" ]; then
    run_b "structural-b/debug-h" "$CARGO_TARGET_DIR/debug/spike-b$exe" structural-b h
  else
    record "structural-b/debug-h" FAIL "no debug binary"
  fi
  verdict
}

sp_unit() {
  run "unit/tests" cargo test --locked --test unit
  run "unit/lib-tests" cargo test --locked --lib unit::
  run "unit/build-release" cargo build --locked --release
  if [ -x "$CARGO_TARGET_DIR/release/spike-b$exe" ]; then
    run_b "unit/release" "$CARGO_TARGET_DIR/release/spike-b$exe" unit
  else
    record "unit/release" FAIL "no release binary"
  fi
}

step="${1:-all}"
case "$step" in
  structural) structural ;;
  b) placement_b ;;
  unit) sp_unit ;;
  all)
    structural
    placement_b
    sp_unit
    ;;
  *)
    echo "usage: gate.sh [structural | b | unit]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike B gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEB($step): PASS"; else echo "SPIKEB($step): FAIL"; fi
exit "$failed"
