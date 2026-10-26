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
#        n = 1 T0 run (graph builder, lattice check, linear scan, a64 emit); prints the report
#     3  `spike-c tier crypto --n 1` and `spike-c tier crypto` (n = 8): crypto with am3
#        compiled and installed (placement (b)) after a warm-up run; output equal to T0 line
#        for line (the kernel checks its own decrypt), then a probe call with a receiver
#        shape no guard admits must end in the "deopt not built" trap (the compiled am3 runs)
#     4  `spike-c tier crypto --n 1 --stress` (GC before every allocation, verifier on):
#        report only. PASS if equal; XFAIL if it ends in a trap (a shape the GC freed drops
#        out of the weak IC entry the guard reads; T0 would re-record it, the compiled code
#        can only deopt, and the deopt entry is month 5); FAIL on any other result
#     SKIP without the v8-v7 checkout (scripts/sync-bench.sh)
#   gate.sh              `gate.sh structural` then `gate.sh am3`
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
  echo "== [am3/stress] $bin tier crypto --n 1 --stress (report only)"
  local out status=0
  out="$("$bin" tier crypto --n 1 --stress 2>&1)" || status=$?
  echo "$out"
  if [ "$status" = 0 ]; then
    record "am3/stress" PASS "output equal to T0 under GC stress"
  elif [ "$status" = 1 ] && grep -q "trap of guard" <<<"$out"; then
    record "am3/stress" XFAIL "deopt not built: $(grep -o 'trap of guard g[0-9]*' <<<"$out" | head -1) (month 5)"
  else
    record "am3/stress" FAIL "exit $status"
  fi
}

step="${1:-all}"
case "$step" in
  structural) structural ;;
  am3) am3 ;;
  all)
    structural
    am3
    ;;
  *)
    echo "usage: gate.sh [structural | am3]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike C gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEC($step): PASS"; else echo "SPIKEC($step): FAIL"; fi
exit "$failed"
