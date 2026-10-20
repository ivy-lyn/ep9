#!/usr/bin/env bash
# SP-GC host gate (M0 week 2; docs/plan/09-milestones.md section 4, docs/plan/03-gc.md).
# Exit 0 only if every step that runs passes.
#
#   1  ep9_platform tests: SP-VA, the spill set of each trampoline, stack bounds
#   2  sp-gc tests: probe cases S1-S7 and a short stress
#   3  10^6 random safepoints per host ABI and seed, 0 missed live objects
#   4  negative controls (no spill, exact-only lookup, no end rule) must miss objects,
#      which shows that step 3 can see a miss
# Extras, run when the host allows them and reported as SKIP otherwise:
#   5  macOS arm64 with Rosetta: steps 1-4 for x86_64-apple-darwin (SysV, and the Win64
#      trampoline through extern "win64")
#   6  Win64 build: every target builds for x86_64-pc-windows-msvc (no-op linker);
#      ep9_spill_win64 disassembles to the expected saves and SEH unwind codes
#
# Env: CARGO_TARGET_DIR (default: spikes/sp-gc/target), SP_GC_SAFEPOINTS (default 1000000),
#      SP_GC_SEEDS (default "1 2 3"), SP_GC_CONTROL_SAFEPOINTS (default 200000),
#      SP_GC_EXTRAS=0 skips steps 5 and 6.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$here"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"
N="${SP_GC_SAFEPOINTS:-1000000}"
SEEDS="${SP_GC_SEEDS:-1 2 3}"
CN="${SP_GC_CONTROL_SAFEPOINTS:-200000}"
EXTRAS="${SP_GC_EXTRAS:-1}"
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

# One target: tests, stress per ABI and seed, controls. On macOS arm64 the x86_64 binaries
# run under Rosetta without any prefix.
gate_target() {
  local t="$1" bin abis abi seed
  run "platform-tests/$t" cargo test --manifest-path "$root/Cargo.toml" -p ep9_platform --target "$t"
  run "probes/$t" cargo test --release --target "$t"
  run "build-stress/$t" cargo build --release --bin stress --target "$t"
  bin="$CARGO_TARGET_DIR/$t/release/stress$exe"
  abis="$("$bin" --list-abis)"
  for abi in $abis; do
    for seed in $SEEDS; do
      run "${N}sp/$t/$abi/seed$seed" "$bin" --abi "$abi" --seed "$seed" --safepoints "$N"
    done
    run "control-no-spill/$t/$abi" "$bin" --abi "$abi" --no-spill --safepoints "$CN" --expect-misses
  done
  run "control-exact-only/$t" "$bin" --scan exact-only --safepoints "$CN" --expect-misses
  run "control-no-end/$t" "$bin" --scan no-end --safepoints "$CN" --expect-misses
}

echo "SP-GC gate: host $host, $N safepoints per ABI and seed (seeds: $SEEDS)"
uptime || true
gate_target "$host"

# ---- extra 5: x86_64-apple-darwin under Rosetta ----
if [ "$EXTRAS" != 0 ] && [ "$host" = aarch64-apple-darwin ]; then
  if arch -x86_64 /usr/bin/true 2>/dev/null &&
    rustup target list --installed | grep -qx x86_64-apple-darwin; then
    gate_target x86_64-apple-darwin
  else
    record "rosetta/x86_64-apple-darwin" SKIP "Rosetta or the x86_64-apple-darwin target is missing"
  fi
fi

# ---- extra 6: Win64 build and disassembly ----
find_llvm() { # tool name -> path or empty
  local t="$1" c
  for c in "$(command -v "$t" || true)" "/opt/homebrew/opt/llvm/bin/$t" "/usr/local/opt/llvm/bin/$t" \
    "$(rustc --print sysroot)/lib/rustlib/$host/bin/$t"; do
    if [ -n "$c" ] && [ -x "$c" ]; then echo "$c"; return; fi
  done
}
win64_disasm_check() {
  local w=x86_64-pc-windows-msvc objdump readobj rlib dir obj="" dis unwind ok=0
  CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=true cargo build --target "$w" --all-targets || return 1
  CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=true cargo build --release --target "$w" --all-targets || return 1
  objdump="$(find_llvm llvm-objdump)"
  readobj="$(find_llvm llvm-readobj)"
  if [ -z "$objdump" ] || [ -z "$readobj" ]; then
    echo "llvm-objdump or llvm-readobj not found"
    return 2
  fi
  # The dev profile has no LTO, so the rlib holds machine code (release rlibs hold bitcode).
  rlib=""
  for f in "$CARGO_TARGET_DIR/$w/debug/deps/"libep9_platform-*.rlib; do
    if [ -z "$rlib" ] || [ "$f" -nt "$rlib" ]; then rlib="$f"; fi
  done
  dir="$(mktemp -d)"
  if ! (cd "$dir" && "$(dirname "$objdump")/llvm-ar" x "$rlib" 2>/dev/null); then
    (cd "$dir" && ar x "$rlib")
  fi
  for o in "$dir"/*.o; do
    if "$objdump" -t "$o" 2>/dev/null | grep -q ' ep9_spill_win64$'; then obj="$o"; fi
  done
  if [ -z "$obj" ]; then
    echo "no object in $rlib defines ep9_spill_win64"
    rm -rf "$dir"
    return 1
  fi
  dis="$("$objdump" -d --no-show-raw-insn -M intel --disassemble-symbols=ep9_spill_win64 "$obj")"
  unwind="$("$readobj" --unwind "$obj")"
  echo "$dis" | sed -n '/<ep9_spill_win64>:/,$p'
  echo "$unwind" | grep -E 'PUSH_NONVOL|ALLOC|SAVE_XMM128|PrologSize'
  rm -rf "$dir"
  for r in rbp rbx rdi rsi r12 r13 r14 r15; do
    echo "$dis" | grep -Eq "push[[:space:]]+$r\$" || { echo "missing push $r"; ok=1; }
    echo "$unwind" | grep -q "PUSH_NONVOL reg=$(echo "$r" | tr '[:lower:]' '[:upper:]')\$" || { echo "missing unwind push $r"; ok=1; }
  done
  for i in 6 7 8 9 10 11 12 13 14 15; do
    echo "$dis" | grep -Eq "movaps.*xmm$i\$" || { echo "missing save xmm$i"; ok=1; }
    echo "$unwind" | grep -q "SAVE_XMM128 reg=XMM$i," || { echo "missing unwind xmm$i"; ok=1; }
  done
  echo "$dis" | grep -Eq 'sub[[:space:]]+rsp, 0xc8$' || { echo "missing sub rsp, 200"; ok=1; }
  echo "$unwind" | grep -q 'ALLOC_LARGE size=200' || { echo "missing unwind alloc 200"; ok=1; }
  return $ok
}
if [ "$EXTRAS" != 0 ]; then
  case "$host" in
  *windows*) record "win64-build" SKIP "Windows host: steps 1-4 ran natively" ;;
  *)
    if rustup target list --installed | grep -qx x86_64-pc-windows-msvc; then
      echo "== [win64-build] build all targets, disassemble ep9_spill_win64"
      set +e
      win64_disasm_check
      rc=$?
      set -e
      case $rc in
      0) record "win64-build" PASS "all targets build; saves and SEH codes match" ;;
      2) record "win64-build" SKIP "llvm-objdump/llvm-readobj not found (build passed)" ;;
      *) record "win64-build" FAIL "see the log above" ;;
      esac
    else
      record "win64-build" SKIP "target x86_64-pc-windows-msvc not installed"
    fi
    ;;
  esac
fi

echo
echo "SP-GC gate summary ($host):"
printf '%s\n' "${results[@]}"
if [ "$failed" != 0 ]; then
  echo "SP-GC gate: FAIL"
  exit 1
fi
echo "SP-GC gate: PASS"
