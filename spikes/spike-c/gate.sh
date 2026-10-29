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
#   gate.sh am3          (plan v9 M4-P1 SPIKEC-1; protocol §2, §3 config C as a correctness run)
#     1  release build (always: cargo decides what is fresh, so the checks never run a binary
#        of older sources; in `all` mode the structural step's build counts, once per run)
#     2  `spike-c compile crypto`: am3 through the whole pipeline from the live facts of an
#        n = 1 T0 run (graph builder, lattice and guard-placement check, linear scan, a64
#        emit); prints the report, the keyed sites and the bytes per exit (SPIKEC-3)
#     3  `spike-c tier crypto --n 1` and `spike-c tier crypto` (n = 8): crypto with am3
#        compiled and installed (placement (b)) after a warm-up run; output equal to T0 line
#        for line (the kernel checks its own decrypt), then a probe call with a receiver
#        shape no guard admits must deopt into T0 (an exit counter of the compiled am3 rises)
#        and print what a T0 run of the same probe prints (month 5, H29)
#     4  `spike-c tier crypto --n 1 --stress` (GC before every allocation, verifier on):
#        PASS only if the output equals T0 (a shape the GC freed drops out of the weak IC
#        entry a guard reads; that guard now deopts into T0, which re-records it). No XFAIL
#     SKIP without the v8-v7 checkout (scripts/sync-bench.sh)
#   gate.sh linsolve     (plan v9 M4-P4 SPIKEC-4; protocol §7 "lin_solve graph build"; plan v10
#                        M5-P2 SPIKEC-6 WI-SC5) kernel 2:
#     1  release build (as am3)
#     2  `spike-c graph navier-stokes`: lin_solve (an inner closure, found by its function id)
#        built from the live facts of an n = 1 T0 run, the graph checked (lattice, guard
#        placement), registers allocated and verified, then emitted (a report line)
#     3  `spike-c tier navier-stokes --n 1` and `--n 12`: lin_solve compiled and installed by
#        its function id after the warm-up (context slots from the CONTEXT export, calls
#        through RuntimeTable::call_any, raw f64 element loads and stores); output equal to T0
#        line for line (the check line digests every cell of the three fields)
#     SKIP without the v8-v7 checkout
#   gate.sh time         (plan v9 M4-P2 SPIKEC-2; protocol §3) the class S timing of crypto on
#     this host: `spike-c time crypto` (15 rounds + 1 warm-up; T0, C, H0, H1 in new processes,
#     rotated order; paired ratios, the median's interval). A REPORT, never a reading: PASS
#     means every run was right, whatever the ratios; FAIL only on a wrong run. Not in `all`
#   gate.sh              `gate.sh structural`, `gate.sh am3`, then `gate.sh linsolve`
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
built="" # this invocation's release build: "" not run yet, else PASS or FAIL
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

# The release build, at most once per invocation (never skipped because a binary exists: it
# may be one of older sources). Returns 0 if it passed.
build_release() {
  if [ -z "$built" ]; then
    echo "== [build-release] cargo build --locked --release"
    local status=0
    cargo build --locked --release || status=$?
    if [ "$status" = 0 ]; then
      built=PASS
      record "build-release" PASS ""
    else
      built=FAIL
      record "build-release" FAIL "exit $status"
    fi
  fi
  [ "$built" = PASS ]
}

structural() {
  run "unit-tests" cargo test --locked
  if ! build_release; then
    record "structural/release" FAIL "release build failed"
  elif [ -x "$CARGO_TARGET_DIR/release/spike-c$exe" ]; then
    "$CARGO_TARGET_DIR/release/spike-c$exe"
    run "structural/release" "$CARGO_TARGET_DIR/release/spike-c$exe" structural all
  else
    record "structural/release" FAIL "no release binary"
  fi
}

am3() {
  if ! build_release; then
    record "am3" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/crypto.js" ]; then
    record "am3" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "am3/compile" "$bin" compile crypto
  run "am3/tier-n1" "$bin" tier crypto --n 1
  run "am3/tier-n8" "$bin" tier crypto
  run "am3/stress" "$bin" tier crypto --n 1 --stress
}

linsolve() {
  if ! build_release; then
    record "linsolve" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/navier-stokes.js" ]; then
    record "linsolve" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "linsolve/graph" "$bin" graph navier-stokes
  run "linsolve/tier-n1" "$bin" tier navier-stokes --n 1
  run "linsolve/tier-n12" "$bin" tier navier-stokes --n 12
}

timing() {
  if ! build_release; then
    record "time" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/crypto.js" ]; then
    record "time" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "time/crypto (report)" "$bin" time crypto
}

step="${1:-all}"
case "$step" in
  structural) structural ;;
  am3) am3 ;;
  linsolve) linsolve ;;
  time) timing ;;
  all)
    structural
    am3
    linsolve
    ;;
  *)
    echo "usage: gate.sh [structural | am3 | linsolve | time]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike C gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEC($step): PASS"; else echo "SPIKEC($step): FAIL"; fi
exit "$failed"
