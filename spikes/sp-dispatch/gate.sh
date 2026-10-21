#!/usr/bin/env bash
# SP-Dispatch host gate (docs/plan/09-milestones.md section 4; H15 ADR,
# docs/adr/H15-interpreter-dispatch.md). Exit 0 only if every structural gate passes and, with
# SP_SPEED=gate, every speed gate passes.
#
# Steps:
#   1. build      stable release (match + sibling), nightly release (+ become), cgoto.c (-O2,
#                 with zig cc 0.17.0 when it is on PATH; see CC below)
#   2. test       cargo test --release (stable; nightly with become)
#   3. check      every variant and cgoto agree with the reference models on small inputs
#   4. deep-loop  threaded variants on a 64 KiB native stack, every opcode in a loop
#   5. disasm     cargo xtask disasm-check --match-loop: the measured binaries (not on Windows),
#                 then staticlib + LLVM IR (args)
#   6. negative   the checks must fail on broken code, for the expected reason (escaped local;
#                 5 handler arguments; on x86-64 a match loop built without tail duplication)
#   7. cross      disasm-check --match-loop on objects for the other installed tier-1 targets
#   8. bench      paired rounds; speed-gate verdicts by the rule of docs/plan/10-quality.md
#                 section 3 (PERF-NOISE): median of the paired ratios, order-statistic interval
#
# Gates. The release expansion is sibling calls on every target (H15, T0 result of 2026-10-21).
#   structural  dsl-3-expansions, tail-exits-<abi>, handler-args, match-loop-<x64 abi>
#   speed       a64: sibling-vs-cgoto-a64, sibling-vs-match-a64
#               x64: sibling-vs-cgoto-<abi>
#   reports     bench prints match-vs-cgoto (every target) and sibling-vs-match (x64); they
#               never count, in any SP_SPEED mode
# match-loop-<x64 abi> stays a blocking structural gate (H15 ADR Rules and CI rule: the
# threaded-loop rule applies to SP-Dispatch, not to T0). LLVM threads this spike's 20-opcode
# loop; the gate keeps the xtask loop check proven on a pass and on a fail (negative build).
#
# Environment:
#   SP_ROUNDS=15                  benchmark rounds (>= 15 on a gate host; >= 31 on a hosted runner)
#   SP_SPEED=gate                 gate: speed verdicts count in the exit code (a gate host in a
#                                 quiet window). report: speed verdicts go to the log,
#                                 bench.json, GitHub warnings and the job summary, but not to the
#                                 exit code (hosted CI runners; H15 ADR)
#   SP_NIGHTLY=nightly-2026-10-04 toolchain for `become`; "none" skips it (dsl-3-expansions: partial)
#   CC=<compiler>                 C compiler for cgoto.c. Unset: `zig cc` when `zig version` prints
#                                 0.17.0 (H15-Q3: one LLVM for cgoto.c on every target; on Windows
#                                 with -target x86_64-windows-msvc, as the Rust binaries), else
#                                 clang. The log line "cc:" says which, and "not pinned" if not zig
#   SP_CROSS=1                    0 skips step 7
#   SP_BENCH=1                    0 skips step 8 (the speed gates then report "not-run")
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
rounds="${SP_ROUNDS:-15}"
speed_mode="${SP_SPEED:-gate}"
nightly="${SP_NIGHTLY:-nightly-2026-10-04}"
case "$speed_mode" in
  gate | report) ;;
  *) echo "SP_SPEED must be gate or report, not '$speed_mode'"; exit 2 ;;
esac
host="$(rustc -vV | sed -n 's/^host: //p')"
zig_pin=0.17.0
if [ -n "${CC:-}" ]; then
  cc=("$CC")
  cc_note="$CC from CC (not pinned)"
elif [ "$(zig version 2>/dev/null)" = "$zig_pin" ]; then
  # zig cc defaults to the gnu ABI on Windows; the measured Rust binaries are msvc.
  cc=(zig cc)
  case "$host" in *windows*) cc+=(-target x86_64-windows-msvc) ;; esac
  cc_note="zig cc $zig_pin (pinned, H15-Q3)"
else
  cc=(clang)
  cc_note="clang (not pinned)"
fi
exe=""
case "$host" in *windows*) exe=".exe" ;; esac
case "$host" in
  aarch64-*) abi="a64" ;;
  x86_64-*windows*) abi="x64-win64" ;;
  x86_64-*) abi="x64-sysv" ;;
  *) abi="$host" ;;
esac
tgt="$here/target"
log="$tgt/gate"
mkdir -p "$log"
rm -f "$log/bench.json"
# Gate results as "id status kind" lines (bash 3.2 has no associative arrays). kind is
# structural or speed. For one id, fail wins over void, then not-run, then partial or
# marginal, then pass.
gates="$log/gates.txt"
: >"$gates"

run() { # run <log-name> <cmd...>: output to the terminal and to target/gate/<log-name>.txt
  local name=$1
  shift
  echo "+ $*"
  "$@" 2>&1 | tee "$log/$name.txt"
  return "${PIPESTATUS[0]}"
}
expect_fail() { # expect_fail <log-name> <reason-ERE> <cmd...>: passes only if the command exits
  # nonzero AND its output matches the reason. Any other failure (missing tool, unrelated
  # compile error, network) fails the negative test.
  local name=$1 reason=$2
  shift 2
  echo "+ (must fail: $reason) $*"
  if "$@" >"$log/$name.txt" 2>&1; then
    echo "  UNEXPECTED PASS (see $log/$name.txt)"
    return 1
  fi
  local hit
  if ! hit="$(grep -m1 -E "$reason" "$log/$name.txt")"; then
    echo "  FAILED FOR ANOTHER REASON (no match for '$reason'; see $log/$name.txt):"
    tail -n 5 "$log/$name.txt" | sed 's/^/    /'
    return 1
  fi
  echo "  failed as expected: $(cut -c1-150 <<<"$hit")"
}
# Called through run/expect_fail ("$@"), which shellcheck cannot see.
# shellcheck disable=SC2329
xtask() { (cd "$root" && cargo xtask disasm-check "$@"); }
# xres <log-name> <RESULT|LOOP>: true if the xtask log has that line with PASS (LOOP: or REPORT,
# the aarch64 case). The tail-exit and argument checks print RESULT; --match-loop prints LOOP.
xres() { grep -qE "^ +$2 [^:]+: (PASS|REPORT)" "$log/$1.txt"; }
# The spike's x86-64 match loop is threaded only because LLVM copies its dispatch jump into
# every arm (tail duplication). Without that, the loop check must fail, for that reason.
notd=(--rustc-arg -Cllvm-args=-disable-tail-duplicate --rustc-arg -Cllvm-args=-disable-early-taildup
  --rustc-arg -Cllvm-args=-tail-dup-placement=false)
loop_neg() { # loop_neg <target>
  expect_fail "neg-match-loop-$1" 'the dispatch is not copied into the arms' \
    xtask --target "$1" --match-loop "${notd[@]}"
}
set_gate() { echo "$1 $2 ${3:-structural}" >>"$gates"; }
load() { uptime 2>/dev/null || echo "n/a (no uptime command; Git Bash on Windows)"; }

echo "== SP-Dispatch gate on $host ($abi); speed gates: $speed_mode"
echo "uptime: $(load)"
# The C compiler and its target (Windows: x86_64-pc-windows-msvc, for zig cc and for clang).
echo "cc: $cc_note: ${cc[*]} ($(command -v "${cc[0]}" || echo "${cc[0]} not found"))"
"${cc[@]}" --version 2>&1 | sed -n '1,2p' | sed 's/^/cc: /'

# 1. build
ok=1
# Explicit --target-dir: an exported CARGO_TARGET_DIR must not move the measured binaries.
run build-stable cargo build --release --manifest-path "$here/Cargo.toml" --target-dir "$tgt" || ok=0
run build-cgoto "${cc[@]}" -O2 -o "$tgt/cgoto$exe" "$here/cgoto.c" || ok=0
have_become=0
if [ "$nightly" != "none" ]; then
  if run build-nightly cargo "+$nightly" build --release --features become \
    --manifest-path "$here/Cargo.toml" --target-dir "$tgt/nightly"; then
    have_become=1
  else
    ok=0
  fi
fi
stable="$tgt/release/sp-dispatch$exe"
nightly_bin="$tgt/nightly/release/sp-dispatch$exe"
cgoto="$tgt/cgoto$exe"
[ "$ok" = 1 ] || { echo "build failed"; exit 1; }
# The x64 speed ratios depend on the CPU model (H15 ADR), so every log names it.
cpu="$("$stable" cpu 2>/dev/null || echo unknown)"
echo "cpu: $cpu"

# 2-4. tests, check, deep-loop
dsl=1
run test-stable cargo test --release --manifest-path "$here/Cargo.toml" --target-dir "$tgt" || dsl=0
become_args=()
if [ "$have_become" = 1 ]; then
  run test-nightly cargo "+$nightly" test --release --features become \
    --manifest-path "$here/Cargo.toml" --target-dir "$tgt/nightly" || dsl=0
  become_args=(--become "$nightly_bin")
fi
run check "$stable" check --cgoto "$cgoto" ${become_args[@]+"${become_args[@]}"} || dsl=0
deep=1
run deep-loop-stable "$stable" deep-loop || deep=0
if [ "$have_become" = 1 ]; then run deep-loop-nightly "$nightly_bin" deep-loop || deep=0; fi
if [ "$dsl" = 1 ] && [ "$have_become" = 1 ]; then set_gate dsl-3-expansions pass
elif [ "$dsl" = 1 ]; then set_gate dsl-3-expansions partial
else set_gate dsl-3-expansions fail; fi

# 5. disasm on the measured binaries (26 handlers per expansion: 20 + illegal + 5 cold)
tail_ok=$deep
args_ok=1
loop_ok=1
case "$host" in
  *windows*)
    # An MSVC-linked .exe has no COFF symbol table (the symbols are in the PDB), so llvm-nm
    # finds no handlers in it. The staticlib check below covers Win64 code.
    echo "SKIP disasm-bin on $host: no symbol table in the .exe; disasm-lib checks Win64" ;;
  *)
    run disasm-bin xtask --expect 26 --match-loop --binary "$stable"
    xres disasm-bin RESULT || tail_ok=0
    xres disasm-bin LOOP || loop_ok=0
    if [ "$have_become" = 1 ]; then run disasm-bin-nightly xtask --expect 26 --binary "$nightly_bin" || tail_ok=0; fi ;;
esac
run disasm-lib xtask --expect 26 --target "$host" --match-loop
xres disasm-lib RESULT || { tail_ok=0; args_ok=0; }
xres disasm-lib LOOP || loop_ok=0
if [ "$have_become" = 1 ]; then
  run disasm-lib-nightly xtask --expect 26 --target "$host" --toolchain "$nightly" --features become \
    || { tail_ok=0; args_ok=0; }
fi

# 6. negative: the checks must catch broken handlers
neg=1
# Reasons: a tail-exit FAIL line of the xtask; std's stack-overflow message; the compile-time
# argument check (E0277); the IR argument check.
expect_fail neg-escape-disasm 'FAIL ep9h__[^ ]+ +insns=' \
  xtask --target "$host" --features neg-escape || neg=0
run build-neg-escape cargo build --release --features neg-escape --manifest-path "$here/Cargo.toml" \
  --target-dir "$tgt/neg" >/dev/null || neg=0
expect_fail neg-escape-deep-loop 'overflowed its stack|stack overflow' \
  "$tgt/neg/release/sp-dispatch$exe" deep-loop || neg=0
expect_fail neg-five-args-build 'error\[E0277\]' cargo build --release --features neg-five-args \
  --manifest-path "$here/Cargo.toml" --target-dir "$tgt/neg" || neg=0
expect_fail neg-five-args-ir 'FAIL ep9h__[^ ]+: 5 parameters' \
  xtask --target "$host" --features neg-five-args,neg-skip-arg-check || neg=0
[ "$neg" = 1 ] || { tail_ok=0; args_ok=0; }
case "$abi" in x64-*) loop_neg "$host" || loop_ok=0 ;; esac
set_gate "tail-exits-$abi" "$([ "$tail_ok" = 1 ] && echo pass || echo fail)"
set_gate handler-args "$([ "$args_ok" = 1 ] && echo pass || echo fail)"
case "$abi" in x64-*) set_gate "match-loop-$abi" "$([ "$loop_ok" = 1 ] && echo pass || echo fail)" ;; esac

# 7. cross: static checks on objects for the other installed tier-1 targets
if [ "${SP_CROSS:-1}" = 1 ]; then
  installed="$(rustup target list --installed)"
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc x86_64-apple-darwin; do
    [ "$t" = "$host" ] && continue
    case "$t" in
      aarch64-*) id="a64" ;;
      *windows*) id="x64-win64" ;;
      *) id="x64-sysv" ;;
    esac
    if ! grep -qx "$t" <<<"$installed"; then
      echo "SKIP $t: rust-std not installed (rustup target add $t)"
      continue
    fi
    run "cross-$t" xtask --expect 26 --target "$t" --match-loop
    set_gate "tail-exits-$id" "$(xres "cross-$t" RESULT && echo pass || echo fail)"
    case "$id" in
      x64-*)
        lok=pass
        xres "cross-$t" LOOP || lok=fail
        loop_neg "$t" || lok=fail
        set_gate "match-loop-$id" "$lok" ;;
    esac
  done
fi

# 8. bench
if [ "${SP_BENCH:-1}" = 1 ]; then
  run bench "$stable" bench --rounds "$rounds" --cgoto "$cgoto" ${become_args[@]+"${become_args[@]}"} \
    --json "$log/bench.json"
  b="$log/bench.txt"
  # The variants must agree on every result (dsl-3-expansions: "build and agree").
  grep -q '^results equal across variants: yes' "$b" || set_gate dsl-3-expansions fail
  n=0
  while read -r _ id v _; do
    set_gate "$id-$abi" "$v" speed
    n=$((n + 1))
  done < <(grep -E '^GATE-RESULT ' "$b")
  # No verdict at all: the bench itself failed (a crash or a missing tool).
  [ "$n" -gt 0 ] || set_gate "bench-$abi" fail
else
  set_gate "sibling-vs-cgoto-$abi" not-run speed
  case "$abi" in a64) set_gate sibling-vs-match-a64 not-run speed ;; esac
fi

echo
echo "uptime: $(load)"
echo "== SP-Dispatch gate summary ($host; cpu: $cpu; speed gates: $speed_mode; logs in $log)"
summary="$(awk '
  function rank(x) { return x == "fail" ? 4 : x == "void" ? 3 : x == "not-run" ? 2 : (x == "partial" || x == "marginal") ? 1 : 0 }
  { if (!($1 in s) || rank($2) > rank(s[$1])) s[$1] = $2; k[$1] = $3 }
  END { for (i in s) printf "  %-28s %-9s %s\n", i, s[i], k[i] }' "$gates" | sort)"
echo "$summary"
structural_bad="$(awk '$3 == "structural" && $2 != "pass"' <<<"$summary")"
speed_bad="$(awk '$3 == "speed" && $2 != "pass"' <<<"$summary")"
if [ "$speed_mode" = report ] && [ -n "$speed_bad" ]; then
  echo "speed gates not passing (SP_SPEED=report: recorded, not in the exit code):"
  echo "$speed_bad"
fi

# GitHub Actions: annotations for every gate that does not pass, and a job summary. The speed
# numbers stay visible even when they do not block (H15 ADR, CI rule).
# Workflow-command escapes: ghm for the message, ghp for a property (title) value.
ghm() { local s=${1//%/%25}; s=${s//$'\r'/%0D}; printf '%s' "${s//$'\n'/%0A}"; }
ghp() { local s; s="$(ghm "$1")"; s=${s//:/%3A}; printf '%s' "${s//,/%2C}"; }
if [ "${GITHUB_ACTIONS:-}" = true ]; then
  while read -r id st _; do
    [ -n "$id" ] || continue
    gid="${id%-"$abi"}"
    geo="$(awk -v g="GATE $gid:" 'index($0, g) == 1 { f = 1; next } f && /geomean/ { sub(/^ +/, ""); print; exit }' \
      "$log/bench.txt" 2>/dev/null)"
    echo "::warning title=$(ghp "SP-Dispatch speed gate $id ($cpu)")::$(ghm "$st, $speed_mode mode. ${geo:-no bench data}. Rounds: $rounds.")"
  done <<<"$speed_bad"
  while read -r id st _; do
    [ -n "$id" ] && echo "::error title=$(ghp "SP-Dispatch gate $id")::$(ghm "$st (structural gate; see the step log)")"
  done <<<"$structural_bad"
fi
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### SP-Dispatch gate: $host ($abi)"
    echo
    echo "CPU: \`$cpu\`. Rounds: $rounds. Speed gates: **$speed_mode**$([ "$speed_mode" = report ] && echo " (recorded, not blocking: hosted runner; PERF-NOISE, H15 ADR)")."
    [ "$rounds" -ge 31 ] || echo "Rounds < 31: below the hosted-runner minimum of 10-quality section 3."
    echo
    echo "| Gate | Status | Kind |"
    echo "|---|---|---|"
    awk '{ printf "| %s | %s | %s |\n", $1, $2, $3 }' <<<"$summary"
    if [ -f "$log/bench.txt" ]; then
      echo
      echo "<details><summary>Speed gates and reports (paired ratios per round)</summary>"
      echo
      echo '```text'
      grep -E '^(cpu:|release expansion:|GATE|REPORT|  (geomean|loop|branchy|call) +n=)' "$log/bench.txt"
      echo '```'
      echo "</details>"
    fi
  } >>"$GITHUB_STEP_SUMMARY"
fi

if [ -n "$structural_bad" ]; then exit 1; fi
if [ "$speed_mode" = gate ] && [ -n "$speed_bad" ]; then exit 1; fi
exit 0
