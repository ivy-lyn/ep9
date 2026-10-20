#!/usr/bin/env bash
# SP-JIT-mem host gate (M0 week 3; docs/plan/09-milestones.md section 4; H28, H27).
# Exit 0 only if every step that runs passes.
#
#   1  ep9_platform tests (the jit and unwind unit tests build under cfg(test))
#   2  ep9_platform clippy -D warnings with --features jit; sp-jit-mem clippy and unit tests
#   3  sp-jit-mem binary: per W^X mode of the host, generated code is called, rewritten,
#      backtraced (std::backtrace and the system unwinder), unwound by a panic, and
#      the toggle cost is measured; a negative control runs without unwind registration
# Extras, run when the host allows them and reported as SKIP otherwise:
#   4  macOS arm64 with Rosetta: steps 1-3 for x86_64-apple-darwin
#   5  non-Windows host: sp-jit-mem and ep9_platform (jit) build for x86_64-pc-windows-msvc
#
# Gate ids (summary at the end): jitmem-<target> (macos-a64, x64-darwin, linux-a64, linux-x64,
# win64), unwind-backtrace/<target>, wx-toggle-cost/<target>.
#
# Env: CARGO_TARGET_DIR (default: spikes/sp-jit-mem/target), SP_JIT_ITERS (bench iterations per
#      batch, default 100000), SP_JIT_EXTRAS=0 skips steps 4 and 5.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$here"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"
ITERS="${SP_JIT_ITERS:-100000}"
EXTRAS="${SP_JIT_EXTRAS:-1}"
host="$(rustc -vV | sed -n 's/^host: //p')"

results=()
failed=0
record() { # id status note
  results+=("$(printf '%-34s %-5s %s' "$1" "$2" "$3")")
  if [ "$2" = FAIL ]; then failed=1; fi
}
run() { # id cmd...: runs cmd, records PASS or FAIL
  local id="$1"
  shift
  echo "== [$id] $*"
  if "$@"; then record "$id" PASS ""; else record "$id" FAIL "exit $?"; fi
}

gate_id() {
  case "$1" in
  aarch64-apple-darwin) echo macos-a64 ;;
  x86_64-apple-darwin) echo x64-darwin ;;
  aarch64-unknown-linux-gnu) echo linux-a64 ;;
  x86_64-unknown-linux-gnu) echo linux-x64 ;;
  x86_64-pc-windows-msvc) echo win64 ;;
  *) echo "$1" ;;
  esac
}

# One target: steps 1-3. On macOS arm64 the x86_64 binaries run under Rosetta without a prefix.
gate_target() {
  local t="$1" id exe="" out bin rc=0 n
  id="$(gate_id "$t")"
  case "$t" in *windows*) exe=".exe" ;; esac
  local before=$failed
  failed=0
  run "platform-tests/$t" cargo test --manifest-path "$root/Cargo.toml" -p ep9_platform --target "$t"
  run "platform-clippy-jit/$t" cargo clippy --manifest-path "$root/Cargo.toml" -p ep9_platform \
    --all-targets --features jit --target "$t" -- -D warnings
  run "spike-clippy/$t" cargo clippy --release --all-targets --target "$t" -- -D warnings
  run "spike-tests/$t" cargo test --release --target "$t"
  run "spike-build/$t" cargo build --release --target "$t"
  bin="$CARGO_TARGET_DIR/$t/release/sp-jit-mem$exe"
  echo "== [spike-run/$t] $bin --iters $ITERS"
  out="$("$bin" --iters "$ITERS" 2>&1)" || rc=$?
  echo "$out"
  if [ "$rc" = 0 ] && echo "$out" | grep -q '^SUMMARY PASS'; then
    record "spike-run/$t" PASS ""
  else
    record "spike-run/$t" FAIL "exit $rc"
  fi

  # unwind-backtrace: every bt-std, bt-raw, callee-saved and panic line of every mode passes.
  n="$(echo "$out" | grep -cE '^RESULT (bt-std|bt-raw|callee-saved|panic)/' || true)"
  if [ "$n" -ge 4 ] && ! echo "$out" | grep -E '^RESULT (bt-std|bt-raw|callee-saved|panic)/' | grep -qv ' PASS '; then
    record "unwind-backtrace/$id" PASS "$n checks"
  else
    record "unwind-backtrace/$id" FAIL "$n checks; see RESULT lines"
  fi
  # wx-toggle-cost: one MEASURE line per mode.
  local modes measures
  modes="$(echo "$out" | grep -cE '^RESULT call/' || true)"
  measures="$(echo "$out" | grep -E '^MEASURE wx-toggle/' || true)"
  if [ "$modes" -ge 1 ] && [ "$(echo "$measures" | grep -c . || true)" = "$modes" ]; then
    record "wx-toggle-cost/$id" PASS "$(echo "$measures" | sed 's/^MEASURE wx-toggle\///' | tr '\n' ';')"
  else
    record "wx-toggle-cost/$id" FAIL "expected $modes MEASURE lines"
  fi

  if [ "$failed" = 0 ]; then record "jitmem-$id" PASS ""; else record "jitmem-$id" FAIL "a step above failed"; fi
  if [ "$before" != 0 ]; then failed=1; fi
}

echo "SP-JIT-mem gate: host $host"
uptime || true
gate_target "$host"

# ---- extra 4: x86_64-apple-darwin under Rosetta ----
if [ "$EXTRAS" != 0 ] && [ "$host" = aarch64-apple-darwin ]; then
  if arch -x86_64 /usr/bin/true 2>/dev/null &&
    rustup target list --installed | grep -qx x86_64-apple-darwin; then
    gate_target x86_64-apple-darwin
  else
    record "jitmem-x64-darwin" SKIP "Rosetta or the x86_64-apple-darwin target is missing"
  fi
fi

# ---- extra 5: Windows build ----
if [ "$EXTRAS" != 0 ]; then
  case "$host" in
  *windows*) ;;
  *)
    w=x86_64-pc-windows-msvc
    if rustup target list --installed | grep -qx "$w"; then
      export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=true
      run "win64-build/platform-jit" cargo build --manifest-path "$root/Cargo.toml" -p ep9_platform \
        --lib --features jit --target "$w"
      run "win64-build/clippy" cargo clippy --manifest-path "$root/Cargo.toml" -p ep9_platform \
        --all-targets --features jit --target "$w" -- -D warnings
      run "win64-build/spike" cargo clippy --release --all-targets --target "$w" -- -D warnings
      unset CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER
    else
      record "win64-build" SKIP "target $w not installed"
    fi
    ;;
  esac
fi

echo
echo "SP-JIT-mem gate summary ($host):"
printf '%s\n' "${results[@]}"
if [ "$failed" != 0 ]; then
  echo "SP-JIT-mem gate: FAIL"
  exit 1
fi
echo "SP-JIT-mem gate: PASS"
