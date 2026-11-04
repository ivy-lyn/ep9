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
#        line for line (the check line digests every cell of the three fields), and loop(n)
#        takes no exit (0 required in every unforced run but am3 under GC stress,
#        `tier::unforced_ok`; Codex M5-P2 r1 spikec6 2)
#     SKIP without the v8-v7 checkout
#   gate.sh navier       (plan v11 M6-P1 SPIKEC-9 §3.8) kernel 2's whole hot set:
#     1  release build (as am3)
#     2  `spike-c graph navier-stokes 0`, `1`, `2`: lin_solve, project and advect each built
#        from the live facts of an n = 1 T0 run, checked, allocated and emitted (pool
#        constants, global loads, calls through call_any, double compares, `x | 0` on a
#        double, Float64 phis, wide jumps, the out-of-line load of Math.sqrt)
#     3  `spike-c tier navier-stokes --all --n 1` and `--n 12`: all three compiled and
#        installed together after the warm-up; output equal to T0 line for line and 0 exits
#        in loop(n). Each guard of the set is forced in `gate.sh diff` (the kernels part)
#     SKIP without the v8-v7 checkout
#   gate.sh deopt        (plan v10 M5-P2 SPIKEC-6 WI-SC4; protocol §4, C2) forced deopts of am3:
#     1  release build (as am3)
#     2  `spike-c deopt crypto`: for each guard of am3, a config C run of crypto (n = 1) with
#        only that guard forced (its countdown word ends at its 3rd pass, emit_a64::ForceTable):
#        output equal to T0, the heap verifier clean after the run, and the VM's exit
#        counters hold exactly that one exit
#     3  `SPIKE_C_FORCE_DEOPT=1 spike-c tier crypto --n 1 --stress`: every guard armed from the
#        seed, GC before every allocation, verifier on: output equal to T0, each guard's
#        counter at least its forced exit, every armed guard forced (`tier::forced_coverage`;
#        Codex M5-P2 r1 spikec6 3), heap verifier clean
#     SKIP without the v8-v7 checkout
#   gate.sh diff         (plan v10 M5-P3 SPIKEC-7 WI-SC6; protocol C3) the tier differential:
#     1  release build (as am3)
#     2  `spike-c diff`: the gate kernels at n = 1 (crypto's am3, navier-stokes' hot set): T0
#        equal to node line for line (node on PATH, else SKIP), config C equal to T0 with 0
#        exits, then forcing: every guard armed from a seed, then each guard not forced yet
#        armed at its first pass, every run equal to T0; then the seeded corpus, seeds 1..200
#        (src/diff.rs: int32 overflow, double/int mixes, holes, out-of-bounds, shape changes,
#        valueOf, DICTIONARY arrays, throws), T0 vs config C byte for byte, unforced and
#        forced the same way; then the corpus on ics_only isolates (MC-w21-9, M6-P3: a guard
#        never reached is named). FAIL on any difference, a refused compile or (feedback
#        full) a guard never forced. No GC-timing compare (MC-w17-10): a lint refuses WeakRef,
#        FinalizationRegistry and gc in every program. SKIP without the v8-v7 checkout
#   gate.sh spfacts      (plan v10 M5-P3 SPFACTS WI-SC8; protocol §6.3, C5c) SP-Facts, config P:
#     1  release build (as am3)
#     2  `spike-c sp-facts write crypto <file>`: a training run (loop(1) in T0) writes the live
#        facts of the kernel's unit as a PROFILED file (FACT_SCHEMA_VERSION 2)
#     3  `spike-c sp-facts check crypto <file> --n 1` and `--n 8`, a new process: the file read
#        and validated (no record dropped), the hot set compiled from it and installed before
#        the first run(), then loop(1) (the start-up exits are counted and printed) and loop(n);
#        every line equal to T0 and to config C, the same set compiled as C, heap verifier clean
#     3b `spike-c sp-facts check crypto <file> --n 1 --feedback ics_only` (MC-w21-9, SPIKEC-8):
#        the same check with T0 and P on isolates of the jit preset with ics_only feedback (T0
#        keeps every IC and drops the profile records, MODES-D19): compiled code from the file
#        runs on an ics_only isolate, every line equal to T0; the start-up exits are printed
#        by cause (MC-w21-5)
#     4  `spike-c sp-facts wrong crypto`: config P with wrong files (another kernel's, another
#        run's, records shuffled across slots, no records, another FACT_SCHEMA_VERSION, a
#        RUNTIME file): output equal to T0 each time, the stale, other-schema and RUNTIME files
#        refused, and compiled code from a wrong file took exits (MC-w15-18: a wrong profile
#        costs speed, never output)
#     SKIP without the v8-v7 checkout. The Mac t(C) / t(P) report is `spike-c sp-facts time
#     crypto` (a report, never a reading; not in any gate step)
#   gate.sh time         (plan v9 M4-P2 SPIKEC-2; protocol §3) the class S timing of crypto on
#     this host: `spike-c time crypto` (15 rounds + 1 warm-up; T0, C, H0, H1 in new processes,
#     rotated order; paired ratios, the median's interval). A REPORT, never a reading: PASS
#     means every run was right, whatever the ratios; FAIL only on a wrong run. Not in `all`
#   gate.sh lazy         (plan v11 M6-P2 SPIKEC-10 §4.8 task 1; H29-Q4) the short lazy-deopt
#     probe: `spike-c lazy` (a compiled call's frame swapped to its lazy exit while the callee
#     runs, the frame-record walk before and after, T0 resumes after the call; output equal to
#     T0). macOS arm64 only (the frame-record walk). A report step: not in `all`
#   gate.sh verdict [--fixtures]  (plan v11 M6-P2 SPIKEC-10 §4.8 task 4; VERDICT-DRY; SCP §8)
#     the Verdict rows: the clean-tree and commit checks, the load before, every step of `all`,
#     C4 (`spike-c time crypto`, `spike-c time navier-stokes`), C5c (`spike-c sp-facts time
#     crypto`), the load after, then `spike-c verdict-rows` prints the SCP §8 table. On a tree
#     that is not clean it prints no table and fails. The orchestrator's step (no agent
#     active; a report before CF). --fixtures: the table of the fixture inputs only (no run)
#   gate.sh              `gate.sh structural`, `gate.sh am3`, `gate.sh deopt`, `gate.sh linsolve`,
#                        `gate.sh navier`, then `gate.sh diff` and `gate.sh spfacts`
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

deopt() {
  if ! build_release; then
    record "deopt" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/crypto.js" ]; then
    record "deopt" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "deopt/each-guard" "$bin" deopt crypto
  run "deopt/seeded-stress" env SPIKE_C_FORCE_DEOPT=1 "$bin" tier crypto --n 1 --stress
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

navier() {
  if ! build_release; then
    record "navier" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/navier-stokes.js" ]; then
    record "navier" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "navier/graph-lin_solve" "$bin" graph navier-stokes 0
  run "navier/graph-project" "$bin" graph navier-stokes 1
  run "navier/graph-advect" "$bin" graph navier-stokes 2
  run "navier/set-n1" "$bin" tier navier-stokes --all --n 1
  run "navier/set-n12" "$bin" tier navier-stokes --all --n 12
}

diff_step() {
  if ! build_release; then
    record "diff" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/crypto.js" ] || [ ! -f "$bench/navier-stokes.js" ]; then
    record "diff" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  run "diff/kernels+corpus" "$bin" diff
}

spfacts() {
  if ! build_release; then
    record "spfacts" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local bench="${SPIKE_C_BENCH_DIR:-$here/../../third_party/bench-v8v7/bench/bench-v8}"
  if [ ! -f "$bench/crypto.js" ]; then
    record "spfacts" SKIP "no v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
    return
  fi
  local prof="$CARGO_TARGET_DIR/spfacts-crypto.facts"
  rm -f "$prof"
  run "spfacts/write" "$bin" sp-facts write crypto "$prof"
  run "spfacts/p-n1" "$bin" sp-facts check crypto "$prof" --n 1
  run "spfacts/p-n8" "$bin" sp-facts check crypto "$prof"
  run "spfacts/p-ics-only" "$bin" sp-facts check crypto "$prof" --n 1 --feedback ics_only
  run "spfacts/wrong" "$bin" sp-facts wrong crypto
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

lazy_step() {
  if ! build_release; then
    record "lazy" FAIL "release build failed"
    return
  fi
  run "lazy/probe (report)" "$CARGO_TARGET_DIR/release/spike-c$exe" lazy
}

# The load averages now (the host's 1, 5 and 15 minute loads).
loads() {
  if command -v sysctl > /dev/null && sysctl -n vm.loadavg > /dev/null 2>&1; then
    sysctl -n vm.loadavg | tr -d '{}' | awk '{print $1, $2, $3}'
  elif [ -r /proc/loadavg ]; then
    awk '{print $1, $2, $3}' /proc/loadavg
  else
    echo "unknown"
  fi
}

verdict() {
  local fixtures="${1:-}"
  if ! build_release; then
    record "verdict" FAIL "release build failed"
    return
  fi
  local bin="$CARGO_TARGET_DIR/release/spike-c$exe"
  local dir="$CARGO_TARGET_DIR/verdict"
  rm -rf "$dir"
  mkdir -p "$dir"
  if [ "$fixtures" = "--fixtures" ]; then
    run "verdict/fixtures" "$bin" verdict-fixtures "$dir"
  else
    local root commit clean=no
    root="$(git -C "$here" rev-parse --show-toplevel)"
    commit="$(git -C "$root" rev-parse HEAD)"
    if [ -z "$(git -C "$root" status --porcelain --untracked-files=no)" ]; then clean=yes; fi
    local before
    before="$(loads)"
    structural
    am3
    deopt
    linsolve
    navier
    diff_step
    spfacts
    for k in crypto navier-stokes; do
      run "verdict/c4-$k" sh -c '"$1" time "$2" > "$3"' _ "$bin" "$k" "$dir/c4-$k.txt"
    done
    run "verdict/c5c" sh -c '"$1" sp-facts time crypto > "$2"' _ "$bin" "$dir/c5c.txt"
    {
      echo "commit=$commit"
      echo "clean=$clean"
      echo "host=$(sed -n 's/^host: \(.*\), load .*/\1/p' "$dir/c4-crypto.txt")"
      echo "load_before=$before"
      echo "load_after=$(loads)"
    } > "$dir/meta.txt"
    for r in "${results[@]}"; do echo "$r"; done > "$dir/steps.txt"
  fi
  echo "== [verdict/rows] $bin verdict-rows $dir"
  if "$bin" verdict-rows "$dir"; then
    record "verdict/rows" PASS "$dir"
  else
    record "verdict/rows" FAIL "no table (see above)"
  fi
}

step="${1:-all}"
case "$step" in
  structural) structural ;;
  am3) am3 ;;
  deopt) deopt ;;
  linsolve) linsolve ;;
  navier) navier ;;
  diff) diff_step ;;
  spfacts) spfacts ;;
  time) timing ;;
  lazy) lazy_step ;;
  verdict) verdict "${2:-}" ;;
  all)
    structural
    am3
    deopt
    linsolve
    navier
    diff_step
    spfacts
    ;;
  *)
    echo "usage: gate.sh [structural | am3 | deopt | linsolve | navier | diff | spfacts | time | lazy | verdict [--fixtures]]" >&2
    exit 2
    ;;
esac

echo
echo "== Spike C gate ($step) on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "SPIKEC($step): PASS"; else echo "SPIKEC($step): FAIL"; fi
exit "$failed"
