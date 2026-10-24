#!/usr/bin/env bash
# H15-Q2 study: the Zen 3 (AMD EPYC 7763) `branchy` gap of the release expansion (sibling
# calls) against cgoto.c (docs/adr/H15-interpreter-dispatch.md, H15-Q2; decision
# M2-P3-SPD-D12). Report mode only: no gate. It prints the CPU model and the per-kernel
# sibling / cgoto.c ratios (paired rounds, median and interval) for the ABI of this host (x64
# SysV on Linux, Win64 on Windows), for 3 builds of cgoto.c, and the T0 (ep9) branchy timing.
#
# Usage: bash spikes/sp-dispatch/zen3-branchy.sh   (no arguments)
# Exit status: 0, unless a build fails (1). A failed or missing measurement is a log line, not
# an error. CI (WIN-STATIC-2) runs it as a report step on linux-x64 and windows-x64 and uploads
# the log (spikes/sp-dispatch/target/zen3/zen3-branchy.log) with the CPU model.
#
# Hypothesis (H15-Q2 fill, PERF3, M3-P2): the gap is in the cgoto.c baseline, not in the sibling
# dispatch. On `branchy` the one data-dependent bytecode branch is JumpIfFalse. cgoto.c writes
# it as `if (acc == 0) pc += off; else pc += 3; DISPATCH();`: the C compiler may make that a
# select (cmov or a plain add) and one shared indirect jump, so the data-dependent choice moves
# from a conditional branch into the target of the next indirect jump. The sibling handler is
# a conditional branch with two tail jumps. Zen 3 may predict (or recover from) the two forms
# at different cost; Zen 4/5 and Intel do not show the gap (H15-Q2 text).
# Test: the same cgoto.c in 3 forms of JUMP_IF_FALSE, each against the same sibling binary:
#   cgoto-as-is   the committed cgoto.c (the gate baseline)
#   cgoto-branch  JUMP_IF_FALSE as a branch with two DISPATCH() jumps (the sibling shape)
#   cgoto-select  JUMP_IF_FALSE as one select, then one DISPATCH() (__builtin_unpredictable)
# If the sibling / cgoto.c ratio on `branchy` drops near 1.0 (below the 1.5 kernel limit) for
# cgoto-branch and stays high for cgoto-select, the gap follows the JUMP_IF_FALSE form of the
# baseline, and the kernel measures a C compiler choice, not T0 dispatch. If all 3 give the
# same high ratio, the hypothesis is false. The script also counts cmov instructions in each
# cgoto build (objdump, when one is on PATH) so the log shows which form the compiler made.
#
# Environment:
#   ZEN3_ROUNDS=31   paired rounds per bench (the hosted-runner minimum, 10-quality section 3)
#   ZEN3_T0=1        0 skips the T0 part (ep9 release build and t0-branchy.js timing)
#   CC               as in gate.sh: unset = zig cc 0.17.0 when present (H15-Q3), else clang
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
rounds="${ZEN3_ROUNDS:-31}"
host="$(rustc -vV | sed -n 's/^host: //p')"
exe=""
case "$host" in *windows*) exe=".exe" ;; esac
case "$host" in
  aarch64-*) abi="a64" ;;
  x86_64-*windows*) abi="x64-win64" ;;
  x86_64-*) abi="x64-sysv" ;;
  *) abi="$host" ;;
esac
# The C compiler, as in gate.sh (H15-Q3 pin).
zig_pin=0.17.0
if [ -n "${CC:-}" ]; then
  cc=("$CC"); cc_note="$CC from CC (not pinned)"
elif [ "$(zig version 2>/dev/null)" = "$zig_pin" ]; then
  cc=(zig cc); cc_note="zig cc $zig_pin (pinned, H15-Q3)"
  case "$host" in *windows*) cc+=(-target x86_64-windows-msvc) ;; esac
else
  cc=(clang); cc_note="clang (not pinned)"
fi

out="$here/target/zen3"
mkdir -p "$out"
log="$out/zen3-branchy.log"
: >"$log"
say() { echo "$*" | tee -a "$log"; }

say "== zen3-branchy (H15-Q2 study, report mode) on $host ($abi)"
say "date: $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
say "uptime: $(uptime 2>/dev/null || echo n/a)"
say "cc: $cc_note: ${cc[*]}"
(cd "$out" && "${cc[@]}" --version 2>&1 | sed -n '1p' | sed 's/^/cc: /') | tee -a "$log"
say "rounds: $rounds"

# 1. Builds. A failed build is exit 1 (the step contract).
if ! cargo build --release --manifest-path "$here/Cargo.toml" --target-dir "$here/target" >>"$log" 2>&1; then
  say "build failed: sp-dispatch (see $log)"; exit 1
fi
sp="$here/target/release/sp-dispatch$exe"
say "cpu: $("$sp" cpu 2>/dev/null || echo unknown)"
src="$here/cgoto.c"
jif='  if (acc == 0) pc += rd_off(pc + 1); else pc += 3;'
if ! grep -qxF "$jif" "$src"; then
  say "cgoto.c changed: the JUMP_IF_FALSE line is not '$jif'; only cgoto-as-is is measured"
fi
# The two variants differ from cgoto.c in the JUMP_IF_FALSE body only (the line above and the
# DISPATCH() after it).
awk -v jif="$jif" '
  $0 == jif { print "  if (acc == 0) { pc += rd_off(pc + 1); DISPATCH(); }"; print "  pc += 3;"; next }
  { print }' "$src" >"$out/cgoto-branch.c"
awk -v jif="$jif" '
  $0 == jif { print "  pc += __builtin_unpredictable(acc == 0) ? rd_off(pc + 1) : 3;"; next }
  { print }' "$src" >"$out/cgoto-select.c"
variants=(as-is)
cmp -s "$src" "$out/cgoto-branch.c" || variants+=(branch)
cmp -s "$src" "$out/cgoto-select.c" || variants+=(select)
for v in "${variants[@]}"; do
  case "$v" in as-is) c="$src" ;; *) c="$out/cgoto-$v.c" ;; esac
  if ! "${cc[@]}" -O2 -o "$out/cgoto-$v$exe" "$c" >>"$log" 2>&1; then
    say "build failed: cgoto-$v (see $log)"; exit 1
  fi
done
t0=0
if [ "${ZEN3_T0:-1}" = 1 ]; then
  if cargo build --release --locked --manifest-path "$root/Cargo.toml" -p ep9_cli >>"$log" 2>&1; then
    t0=1
  else
    say "build failed: ep9 (T0)"; exit 1
  fi
fi

# 2. The JUMP_IF_FALSE form in each cgoto build (best effort): the select and indirect-jump
# counts of the whole binary. The 3 builds differ in JUMP_IF_FALSE only, so a difference
# between them is that handler: one more indirect jump = two dispatch jumps (branch form);
# one more cmov/csel = a select.
dis=""
for d in llvm-objdump objdump; do command -v "$d" >/dev/null 2>&1 && { dis=$d; break; }; done
for v in "${variants[@]}"; do
  if [ -n "$dis" ] && "$dis" -d "$out/cgoto-$v$exe" >"$out/cgoto-$v.dis" 2>/dev/null; then
    sel=$(grep -ciE '[[:space:]](cmov[a-z]*|csel|csinc)[[:space:]]' "$out/cgoto-$v.dis")
    ind=$(grep -ciE '[[:space:]](jmpq?[[:space:]]+\*|br[[:space:]]+x)' "$out/cgoto-$v.dis")
    say "form cgoto-$v: $sel selects (cmov/csel/csinc), $ind indirect jumps in the binary ($dis)"
  else
    say "form cgoto-$v: not read (no objdump)"
  fi
done

# 3. Bench: sibling (and match) against each cgoto build, paired rounds (sp-dispatch bench).
for v in "${variants[@]}"; do
  b="$out/bench-$v.txt"
  if "$sp" bench --rounds "$rounds" --cgoto "$out/cgoto-$v$exe" --json "$out/bench-$v.json" >"$b" 2>>"$log"; then
    :
  else
    say "bench cgoto-$v: exit $? (see $b)"
  fi
  cat "$b" >>"$log"
  # Per-kernel paired ratio of the release gate (sibling-vs-cgoto), one line per kernel.
  awk -v v="$v" -v abi="$abi" '
    /^GATE sibling-vs-cgoto:/ { g = 1; next }
    g && /^  [a-z]+ +n=/ { printf "RATIO %s cgoto-%s sibling/cgoto %s %s %s %s %s %s %s %s\n", abi, v, $1, $3, $4, $5, $6, $7, $8, $9 }
    g && /^GATE-RESULT/ { sub(/^GATE-RESULT [^ ]+ /, ""); print "VERDICT " abi " cgoto-" v " " $0; g = 0 }' "$b" |
    tee -a "$log"
done

# 4. T0: ep9 on t0-branchy.js and t0-loop.js, sibling and match expansions, wall time (ms),
# interleaved. Not a gate: the T0 kernels are JS, not the spike's bytecode, so there is no
# cgoto.c twin; the match / sibling ratio on this CPU says whether the T0 sibling dispatch is
# itself slow on branchy code here.
if [ "$t0" = 1 ]; then
  ep9="$root/target/release/ep9$exe"
  [ -n "${CARGO_TARGET_DIR:-}" ] && ep9="$CARGO_TARGET_DIR/release/ep9$exe"
  ms() { # ms <cmd...>: wall time of one run in ms (bash SECONDS has 1 s steps; use date +%s%N or perl)
    local s e
    s=$(perl -MTime::HiRes=time -e 'printf "%.3f", time*1000' 2>/dev/null || echo 0)
    "$@" >/dev/null 2>&1 || { echo fail; return; }
    e=$(perl -MTime::HiRes=time -e 'printf "%.3f", time*1000' 2>/dev/null || echo 0)
    awk -v s="$s" -v e="$e" 'BEGIN { printf "%.1f", e - s }'
  }
  n_t0=$(( rounds < 11 ? rounds : 11 ))
  for k in t0-branchy t0-loop; do
    f="$root/tools/bench/workloads/$k.js"
    sib=() mat=()
    for ((i = 0; i < n_t0; i++)); do
      if ((i % 2)); then
        mat+=("$(ms "$ep9" --dispatch match "$f")"); sib+=("$(ms "$ep9" --dispatch sibling "$f")")
      else
        sib+=("$(ms "$ep9" --dispatch sibling "$f")"); mat+=("$(ms "$ep9" --dispatch match "$f")")
      fi
    done
    med() { printf '%s\n' "$@" | grep -v fail | sort -n | awk '{ a[NR] = $1 } END { if (NR) print a[int((NR + 1) / 2)]; else print "n/a" }'; }
    ms_s=$(med "${sib[@]}"); ms_m=$(med "${mat[@]}")
    ratio=$(awk -v m="$ms_m" -v s="$ms_s" 'BEGIN { if (s > 0 && m > 0) printf "%.3f", m / s; else print "n/a" }')
    say "T0 $abi $k sibling ${ms_s} ms, match ${ms_m} ms (median of $n_t0), match/sibling $ratio"
  done
fi

say "uptime: $(uptime 2>/dev/null || echo n/a)"
say "== summary (cpu: $("$sp" cpu 2>/dev/null || echo unknown); abi $abi; log $log)"
grep -E '^(RATIO|VERDICT|form|T0) ' "$log" | sed 's/^/  /'
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### zen3-branchy (H15-Q2 report): $host ($abi)"
    echo
    echo "CPU: \`$("$sp" cpu 2>/dev/null || echo unknown)\`. Rounds: $rounds. cc: $cc_note."
    echo
    echo '```text'
    grep -E '^(RATIO|VERDICT|form|T0) ' "$log"
    echo '```'
  } >>"$GITHUB_STEP_SUMMARY"
fi
exit 0
