#!/usr/bin/env bash
# SP-Dispatch host gate (docs/plan/09-milestones.md section 4). Exit 0 only if every gate passes.
#
# Steps:
#   1. build      stable release (match + sibling), nightly release (+ become), cgoto.c (clang -O2)
#   2. test       cargo test --release (stable; nightly with become)
#   3. check      every variant and cgoto agree with the reference models on small inputs
#   4. deep-loop  threaded variants on a 64 KiB native stack, every opcode in a loop
#   5. disasm     cargo xtask disasm-check: the measured binaries (not on Windows), then
#                 staticlib + LLVM IR (args)
#   6. negative   the checks must fail on broken code, for the expected reason (escaped local;
#                 5 handler arguments)
#   7. cross      disasm-check on objects for the other installed tier-1 targets (static only)
#   8. bench      interleaved medians: sibling <= 1.25x cgoto, >= 1.2x faster than match
#
# Environment:
#   SP_ROUNDS=15                  benchmark rounds (median of these; >= 10 for the gate)
#   SP_NIGHTLY=nightly-2026-10-04 toolchain for `become`; "none" skips it (dsl-3-expansions: partial)
#   CC=clang                      C compiler for cgoto.c
#   SP_CROSS=1                    0 skips step 7
#   SP_BENCH=1                    0 skips step 8 (the speed gates then report "not-run")
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
rounds="${SP_ROUNDS:-15}"
nightly="${SP_NIGHTLY:-nightly-2026-10-04}"
cc="${CC:-clang}"
host="$(rustc -vV | sed -n 's/^host: //p')"
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
# Gate results as "id status" lines (bash 3.2 has no associative arrays). For one id,
# fail wins over not-run, then partial, then pass.
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
set_gate() { echo "$1 $2" >>"$gates"; }

echo "== SP-Dispatch gate on $host ($abi)"
echo "uptime: $(uptime)"

# 1. build
ok=1
# Explicit --target-dir: an exported CARGO_TARGET_DIR must not move the measured binaries.
run build-stable cargo build --release --manifest-path "$here/Cargo.toml" --target-dir "$tgt" || ok=0
run build-cgoto "$cc" -O2 -o "$tgt/cgoto$exe" "$here/cgoto.c" || ok=0
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
case "$host" in
  *windows*)
    # An MSVC-linked .exe has no COFF symbol table (the symbols are in the PDB), so llvm-nm
    # finds no handlers in it. The staticlib check below covers Win64 code.
    echo "SKIP disasm-bin on $host: no symbol table in the .exe; disasm-lib checks Win64" ;;
  *)
    run disasm-bin xtask --expect 26 --binary "$stable" || tail_ok=0
    if [ "$have_become" = 1 ]; then run disasm-bin-nightly xtask --expect 26 --binary "$nightly_bin" || tail_ok=0; fi ;;
esac
run disasm-lib xtask --expect 26 --target "$host" || { tail_ok=0; args_ok=0; }
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
set_gate "tail-exits-$abi" "$([ "$tail_ok" = 1 ] && echo pass || echo fail)"
set_gate handler-args "$([ "$args_ok" = 1 ] && echo pass || echo fail)"

# 7. cross: static checks on objects for the other installed tier-1 targets
if [ "${SP_CROSS:-1}" = 1 ]; then
  installed="$(rustup target list --installed)"
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc x86_64-apple-darwin; do
    [ "$t" = "$host" ] && continue
    case "$t" in
      aarch64-*) id="tail-exits-a64" ;;
      *windows*) id="tail-exits-x64-win64" ;;
      *) id="tail-exits-x64-sysv" ;;
    esac
    if ! grep -qx "$t" <<<"$installed"; then
      echo "SKIP $t: rust-std not installed (rustup target add $t)"
      continue
    fi
    if run "cross-$t" xtask --expect 26 --target "$t"; then
      set_gate "$id" pass
    else
      set_gate "$id" fail
    fi
  done
fi

# 8. bench
if [ "${SP_BENCH:-1}" = 1 ]; then
  run bench "$stable" bench --rounds "$rounds" --cgoto "$cgoto" ${become_args[@]+"${become_args[@]}"}
  b="$log/bench.txt"
  g1="$(grep -m1 'GATE sibling <= 1.25x cgoto' "$b" | awk '{print $NF}')"
  g2="$(grep -m1 'GATE sibling >= 1.2x faster than match' "$b" | awk '{print $NF}')"
  eq="$(grep -m1 'results equal across variants' "$b" | awk '{print $NF}')"
  [ "$eq" = yes ] || { g1=FAIL; g2=FAIL; }
  set_gate "sibling-vs-cgoto-$abi" "$([ "$g1" = PASS ] && echo pass || echo fail)"
  set_gate "sibling-vs-match-$abi" "$([ "$g2" = PASS ] && echo pass || echo fail)"
else
  set_gate "sibling-vs-cgoto-$abi" not-run
  set_gate "sibling-vs-match-$abi" not-run
fi

echo
echo "uptime: $(uptime)"
echo "== SP-Dispatch gate summary ($host; logs in $log)"
summary="$(awk '
  function rank(x) { return x == "fail" ? 3 : x == "not-run" ? 2 : x == "partial" ? 1 : 0 }
  { if (!($1 in s) || rank($2) > rank(s[$1])) s[$1] = $2 }
  END { for (k in s) printf "  %-26s %s\n", k, s[k] }' "$gates" | sort)"
echo "$summary"
if grep -qvE ' pass$' <<<"$summary"; then exit 1; fi
exit 0
