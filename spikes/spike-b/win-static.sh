#!/usr/bin/env bash
# Spike B Win64 static check (P4-WIN-STATIC-1; H28): the UNWIND_INFO that the x86-64 JIT code
# of placement (a) and placement (b) carries (M3-P0 P0-B-X64), read back by LLVM's tools, on a
# host that is not Windows (made for mac a64).
# The real Windows run is tests/win64_unwind.rs on the CI windows-x64 runner. Exit 0 only if
# every step that runs passes.
#
#   1  build tests/win64_unwind.rs for the host with `--cfg win64_static` (`cargo rustc`: only
#      the test crate gets the cfg; a normal build compiles the file to nothing)
#   2  run it: x86-64 code of `drive` and `id` (SpikeEmitter::new(Arch::X86_64)) and of `hold`
#      (jit::hold_probe), then the placement (b) code of the same three (BEmitter::new,
#      jit_b::hold_probe; named bdrive, bid, bhold), made on this host, go into a hand-built
#      COFF object (.text, .xdata with each UNWIND_INFO, .pdata with one RUNTIME_FUNCTION
#      each) plus functions.txt
#   3  `llvm-readobj --unwind` (the toolchain's llvm-tools) on that object equals the golden
#      below (6 RuntimeFunctions); each EndAddress equals the function's code length
#   4  control: `llvm-mc -triple=x86_64-pc-windows-msvc` assembles each prologue shape with
#      `.seh_*` directives and its epilogue (`ret` for (a); `rex64 jmp qword ptr [rdi +
#      8*rax]` for (b)); its unwind codes equal the JIT's, and its bytes equal the JIT
#      prologue and epilogue bytes. Then `llvm-objdump -d` of the (b) functions: no `ret`
#      anywhere, and the exit jump is the only indirect jump. SKIP when no llvm-mc is found
#   5  `cargo xwin build --tests` and `cargo xwin clippy` for x86_64-pc-windows-msvc (an early
#      warning: it compiles the Windows test; it cannot run it). SKIP without cargo-xwin
#
# Env: CARGO_TARGET_DIR (default: spikes/spike-b/target), LLVM_MC (the llvm-mc to use).
# Output: $CARGO_TARGET_DIR/win-static/.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"
host="$(rustc -vV | sed -n 's/^host: //p')"
win=x86_64-pc-windows-msvc

# Step 2 runs a host binary. Never run an x86_64-apple-darwin binary under Rosetta.
if [ "$host" = x86_64-apple-darwin ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
  echo "win-static.sh: x86_64-apple-darwin under Rosetta; run it with the native toolchain" >&2
  exit 2
fi

results=()
failed=0
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

out="$CARGO_TARGET_DIR/win-static"
rm -rf "$out"
mkdir -p "$out"
sysbin="$(rustc --print sysroot)/lib/rustlib/$host/bin"
tool() { # name: the toolchain's llvm-tools copy, then PATH
  if [ -x "$sysbin/$1" ]; then echo "$sysbin/$1"; else command -v "$1" || true; fi
}
readobj="$(tool llvm-readobj)"
objcopy="$(tool llvm-objcopy)"
objdump="$(tool llvm-objdump)"

# The `llvm-readobj --unwind` text of win64-jit.obj. EndAddress offsets are replaced by LEN;
# step 3 checks them against the code lengths in functions.txt.
golden() {
  cat <<'EOF'
Format: COFF-x86-64
Arch: x86_64
AddressSize: 64bit
UnwindInformation [
  RuntimeFunction {
    StartAddress: drive (0x0)
    EndAddress: drive +LEN (0x4)
    UnwindInfoAddress: .xdata (0x8)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 10
      FrameRegister: RBP (0x5)
      FrameOffset: 0x1
      UnwindCodeCount: 4
      UnwindCodes [
        0x0A: SET_FPREG reg=RBP, offset=0x10
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
  RuntimeFunction {
    StartAddress: id (0xC)
    EndAddress: id +LEN (0x10)
    UnwindInfoAddress: .xdata +0xC (0x14)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 10
      FrameRegister: RBP (0x5)
      FrameOffset: 0x1
      UnwindCodeCount: 4
      UnwindCodes [
        0x0A: SET_FPREG reg=RBP, offset=0x10
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
  RuntimeFunction {
    StartAddress: hold (0x18)
    EndAddress: hold +LEN (0x1C)
    UnwindInfoAddress: .xdata +0x18 (0x20)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 13
      FrameRegister: RBP (0x5)
      FrameOffset: 0x2
      UnwindCodeCount: 6
      UnwindCodes [
        0x0D: SET_FPREG reg=RBP, offset=0x20
        0x08: PUSH_NONVOL reg=R12
        0x06: PUSH_NONVOL reg=RBX
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
  RuntimeFunction {
    StartAddress: bdrive (0x24)
    EndAddress: bdrive +LEN (0x28)
    UnwindInfoAddress: .xdata +0x28 (0x2C)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 10
      FrameRegister: RBP (0x5)
      FrameOffset: 0x1
      UnwindCodeCount: 4
      UnwindCodes [
        0x0A: SET_FPREG reg=RBP, offset=0x10
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
  RuntimeFunction {
    StartAddress: bid (0x30)
    EndAddress: bid +LEN (0x34)
    UnwindInfoAddress: .xdata +0x34 (0x38)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 10
      FrameRegister: RBP (0x5)
      FrameOffset: 0x1
      UnwindCodeCount: 4
      UnwindCodes [
        0x0A: SET_FPREG reg=RBP, offset=0x10
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
  RuntimeFunction {
    StartAddress: bhold (0x3C)
    EndAddress: bhold +LEN (0x40)
    UnwindInfoAddress: .xdata +0x40 (0x44)
    UnwindInfo {
      Version: 1
      Flags [ (0x0)
      ]
      PrologSize: 13
      FrameRegister: RBP (0x5)
      FrameOffset: 0x2
      UnwindCodeCount: 6
      UnwindCodes [
        0x0D: SET_FPREG reg=RBP, offset=0x20
        0x08: PUSH_NONVOL reg=R12
        0x06: PUSH_NONVOL reg=RBX
        0x05: PUSH_NONVOL reg=R13
        0x03: PUSH_NONVOL reg=R15
        0x01: PUSH_NONVOL reg=RBP
      ]
    }
  }
]
EOF
}

# The control prologues, by shape "<pushes (Win64 register numbers)>/<lea rbp displacement>/
# <exit>" (functions.txt columns 4, 5 and 10): the same frame written with .seh_* directives,
# and the canonical epilogue the Win64 unwinder recognizes: lea rsp, [rbp - d]; the pops; then
# `ret`, or for placement (b) the indirect jump with REX.W (the form LLVM emits for a Win64
# indirect tail call: the unwinder takes an indirect `jmp` as an epilogue end only with REX.W
# or as `jmp [rip + disp32]`).
control_source() { # shape
  local pro epi setframe exit
  case "${1##*/}" in
    ret) exit="ret" ;;
    jmp) exit="rex64 jmp qword ptr [rdi + 8*rax]" ;;
    *) return 1 ;;
  esac
  case "${1%/*}" in
    5,15,13/16)
      pro="push rbp|.seh_pushreg rbp|push r15|.seh_pushreg r15|push r13|.seh_pushreg r13"
      epi="lea rsp, [rbp - 16]|pop r13|pop r15|pop rbp|$exit"
      setframe="lea rbp, [rsp + 16]|.seh_setframe rbp, 16"
      ;;
    5,15,13,3,12/32)
      pro="push rbp|.seh_pushreg rbp|push r15|.seh_pushreg r15|push r13|.seh_pushreg r13"
      pro="$pro|push rbx|.seh_pushreg rbx|push r12|.seh_pushreg r12"
      epi="lea rsp, [rbp - 32]|pop r12|pop rbx|pop r13|pop r15|pop rbp|$exit"
      setframe="lea rbp, [rsp + 32]|.seh_setframe rbp, 32"
      ;;
    *) return 1 ;;
  esac
  printf '%s\n' "$pro|$setframe|.seh_endprologue|$epi" |
    tr '|' '\n' |
    sed 's/^/\t/' |
    { printf '\t.intel_syntax noprefix\n\t.text\n\t.seh_proc ctl\nctl:\n'; cat; printf '\t.seh_endproc\n'; }
}

# The UnwindInfo { ... } block of function $2 in the llvm-readobj --unwind text $1.
unwind_block() {
  awk -v n="$2" '
    /StartAddress: / { cur = $2 }
    /^    UnwindInfo \{/ { on = (cur == n) }
    on { print }
    on && /^    }$/ { on = 0 }
  ' "$1"
}

hexbytes() { od -An -tx1 -v "$1" | tr -d ' \n'; }

# ---- 1-2: the object ------------------------------------------------------------------------
exe=""
build() {
  local log="$out/build.json"
  cargo rustc --locked --test win64_unwind --message-format=json-render-diagnostics \
    -- --cfg win64_static --check-cfg 'cfg(win64_static)' >"$log" || return 1
  exe="$(grep '"name":"win64_unwind"' "$log" | grep -o '"executable":"[^"]*"' | tail -1 |
    sed 's/^"executable":"//; s/"$//')"
  [ -n "$exe" ] && [ -x "$exe" ] || {
    echo "no win64_unwind test executable in $log" >&2
    return 1
  }
}
dump() {
  EP9_WIN64_STATIC_OUT="$out" "$exe" --nocapture &&
    [ -s "$out/win64-jit.obj" ] && [ -s "$out/functions.txt" ] &&
    cat "$out/functions.txt"
}
run "build-host-test" build
if [ -n "$exe" ]; then run "emit-x64-coff" dump; else record "emit-x64-coff" FAIL "no test binary"; fi

# ---- 3: llvm-readobj against the golden -----------------------------------------------------
readobj_check() {
  # `run` calls this in an `if`, where set -e is off: each command checks its own status.
  "$readobj" --unwind "$out/win64-jit.obj" >"$out/readobj.txt" 2>"$out/readobj.err" || return 1
  if [ -s "$out/readobj.err" ]; then
    echo "llvm-readobj wrote to stderr:" >&2
    cat "$out/readobj.err" >&2
    return 1
  fi
  # Each EndAddress is the function's code length (functions.txt column 2).
  local name len ok=0 got
  while read -r name len _; do
    got="$(sed -n "s/^ *EndAddress: $name +0x\([0-9A-F]*\) .*/\1/p" "$out/readobj.txt")"
    if [ -z "$got" ] || [ "$((16#$got))" != "$len" ]; then
      echo "$name: EndAddress +0x${got:-?}, code length $len" >&2
      ok=1
    fi
  done <"$out/functions.txt"
  sed -n '/^Format:/,$p' "$out/readobj.txt" |
    sed 's/^\( *EndAddress: [a-z]*\) +0x[0-9A-F]* /\1 +LEN /' >"$out/readobj.norm"
  golden >"$out/golden.txt"
  if ! diff -u "$out/golden.txt" "$out/readobj.norm"; then
    echo "llvm-readobj --unwind differs from the golden (above: - golden, + now)" >&2
    ok=1
  fi
  [ "$ok" = 0 ] && echo "llvm-readobj --unwind = golden ($(wc -l <"$out/functions.txt" | tr -d ' ') functions)"
}
if [ -z "$readobj" ]; then
  record "readobj-golden" FAIL "no llvm-readobj (rustup component add llvm-tools)"
elif [ ! -s "$out/functions.txt" ]; then
  record "readobj-golden" FAIL "no object"
else
  run "readobj-golden" readobj_check
fi

# ---- 4: the llvm-mc control -----------------------------------------------------------------
mc="${LLVM_MC:-}"
if [ -z "$mc" ]; then
  mc="$(tool llvm-mc)"
  for d in /opt/homebrew/opt/llvm/bin /usr/local/opt/llvm/bin; do
    if [ -z "$mc" ] && [ -x "$d/llvm-mc" ]; then mc="$d/llvm-mc"; fi
  done
fi
control_check() {
  local name pushes disp pro epi exit shape ok=0 n=0
  while read -r name _ _ pushes disp _ pro epi _ exit; do
    shape="$pushes/$disp/$exit"
    local s="$out/control-$name.s" o="$out/control-$name.obj"
    if ! control_source "$shape" >"$s"; then
      echo "$name: no control prologue for shape $shape" >&2
      ok=1
      continue
    fi
    if ! "$mc" -triple=x86_64-pc-windows-msvc -filetype=obj "$s" -o "$o" ||
      ! "$readobj" --unwind "$o" >"$o.txt"; then
      echo "$name: llvm-mc or llvm-readobj failed on $s" >&2
      ok=1
      continue
    fi
    if ! diff -u <(unwind_block "$o.txt" ctl) <(unwind_block "$out/readobj.txt" "$name"); then
      echo "$name: unwind codes differ from llvm-mc's .seh_* for shape $shape (- llvm-mc, + JIT)" >&2
      ok=1
    fi
    if [ -z "$(unwind_block "$o.txt" ctl)" ]; then
      echo "$name: no UnwindInfo in the control object" >&2
      ok=1
    fi
    if ! "$objcopy" --dump-section ".text=$o.text" "$o" "$o.copy"; then
      echo "$name: llvm-objcopy failed on $o" >&2
      ok=1
      continue
    fi
    local want got
    want="$(hexbytes "$o.text")"
    got="$pro$epi"
    if [ "$want" != "$got" ]; then
      echo "$name: prologue+epilogue bytes $got, llvm-mc $want" >&2
      ok=1
    fi
    n=$((n + 1))
  done <"$out/functions.txt"
  # Placement (b): the whole code of each b* function, disassembled. No `ret` at any
  # instruction, and its one indirect jump is the exit jump.
  local dis="$out/disasm-b.txt" body nb=0
  if ! "$objdump" -d --no-show-raw-insn -M intel "$out/win64-jit.obj" >"$dis"; then
    echo "llvm-objdump failed on win64-jit.obj" >&2
    ok=1
  fi
  while read -r name _; do
    case "$name" in b*) ;; *) continue ;; esac
    body="$(awk -v n="<$name>:" '$2 == n { on = 1; next } /^[0-9a-f]+ <.*>:$/ { on = 0 } on' "$dis")"
    if [ -z "$body" ]; then
      echo "$name: not in the llvm-objdump output" >&2
      ok=1
      continue
    fi
    if grep -qwE 'ret[lq]?' <<<"$body"; then
      echo "$name: a ret in placement (b) code:" >&2
      grep -wE 'ret[lq]?' <<<"$body" >&2
      ok=1
    fi
    if [ "$(grep -cE 'jmp[[:space:]]+qword ptr' <<<"$body")" != 1 ] ||
      ! grep -qF 'jmp	qword ptr [rdi + 8*rax]' <<<"$body"; then
      echo "$name: want exactly one indirect jump, jmp qword ptr [rdi + 8*rax]:" >&2
      grep -E 'jmp[[:space:]]+qword' <<<"$body" >&2
      ok=1
    fi
    nb=$((nb + 1))
  done <"$out/functions.txt"
  if [ "$nb" != 3 ]; then
    echo "$nb placement (b) functions in functions.txt, want 3" >&2
    ok=1
  fi
  [ "$ok" = 0 ] && echo "llvm-mc control: $n functions, unwind codes and prologue/epilogue bytes equal; llvm-objdump: no ret in the $nb (b) functions, one exit jump each"
}
if [ -z "$mc" ] || [ ! -x "$mc" ]; then
  record "llvm-mc-control" SKIP "no llvm-mc (set LLVM_MC)"
elif [ ! -s "$out/readobj.txt" ] || [ -z "$objcopy" ] || [ -z "$objdump" ]; then
  record "llvm-mc-control" FAIL "no readobj output, no llvm-objcopy or no llvm-objdump"
else
  echo "llvm-mc: $mc ($("$mc" --version | sed -n 's/.*LLVM version /LLVM /p' | head -1))"
  run "llvm-mc-control" control_check
fi

# ---- 5: cross-build of the Windows test (early warning) -------------------------------------
case "$host" in
  *windows*) record "xwin" SKIP "Windows host: cargo test runs the real test" ;;
  *)
    if command -v cargo-xwin >/dev/null && rustup target list --installed | grep -qx "$win"; then
      run "xwin-build-tests" cargo xwin build --locked --tests --target "$win"
      run "xwin-clippy" cargo xwin clippy --locked --all-targets --target "$win" -- -D warnings
    else
      record "xwin" SKIP "no cargo-xwin or no $win target"
    fi
    ;;
esac

echo
echo "== Spike B Win64 static check on $host"
for r in "${results[@]}"; do echo "$r"; done
if [ "$failed" = 0 ]; then echo "WIN-STATIC: PASS"; else echo "WIN-STATIC: FAIL"; fi
exit "$failed"
