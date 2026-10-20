#!/usr/bin/env bash
# Barrier fast-path disassembly check (SP-Value, H11; 09-milestones section 4).
#
# Builds ep9_heap with release code generation and checks the instructions of
# `ep9_probe_barrier` (= Slot::set) on each target:
#
#   aarch64:  cell test (and, tst, b.ne)  + ldrb, ldrb, cmp, b.ls  + str     budget: 4 + cell test
#   x86_64:   cell test (movabs, test, jne) + movzx, movzx, cmp, jbe + mov   (SysV and Win64)
#
# The cell test may take at most 3 instructions (2 with the mask in a pinned register, which
# a Rust function cannot have). The 4 header-check instructions must follow it directly, and
# the store must be one plain store. Frame-pointer setup (push rbp / mov rbp, rsp / pop rbp,
# macOS x86_64) is not part of the barrier and is not counted.
#
#   crates/ep9_heap/disasm-barrier.sh [target ...]     default: the host target
#
# Exit 0 if every target passes. Env: CARGO_TARGET_DIR (default: target/disasm-barrier).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/disasm-barrier}"
host="$(rustc -vV | sed -n 's/^host: //p')"
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then targets=("$host"); fi

fail=0
for t in "${targets[@]}"; do
  echo "== $t"
  # dev-opt = release code generation without LTO, so the rlib's own code is emitted.
  # One codegen unit gives one .s file.
  cargo rustc --manifest-path "$root/Cargo.toml" -p ep9_heap --lib --profile dev-opt \
    --target "$t" --quiet -- --emit asm -C codegen-units=1
  asm="$(ls -t "$CARGO_TARGET_DIR/$t/dev-opt/deps/"ep9_heap-*.s | head -1)"
  # The body of ep9_probe_barrier (macOS adds a leading underscore), up to .cfi_endproc or
  # the next function.
  body="$(awk '
    /^_?ep9_probe_barrier:/ { on = 1; next }
    on && (/\.cfi_endproc/ || /^\.seh_endproc/ || /^[[:space:]]*\.(globl|def)[[:space:]]/) { exit }
    on { print }
  ' "$asm")"
  if [ -z "$body" ]; then
    echo "ep9_probe_barrier not found in $asm"
    fail=1
    continue
  fi
  echo "$body" | grep -Ev '^[[:space:]]*(\.|#|;|//)' | sed 's/^/    /'
  # Mnemonics only: no labels, directives or comments; drop frame-pointer setup.
  mn="$(echo "$body" | grep -Ev '^[[:space:]]*($|\.|#|;|//)|:[[:space:]]*$|^[A-Za-z_.$0-9]+:' |
    awk '{print $1, $2, $3}' |
    grep -Ev '^(pushq %rbp|movq %rsp, %rbp|popq %rbp|push rbp|mov rbp, rsp|pop rbp)' |
    awk '{print $1}')"
  case "$t" in
  aarch64-*) first="ldrb"; seq="ldrb ldrb cmp b.ls|b.hi"; store="str" ;;
  x86_64-*) first="movzbl"; seq="movzbl movzbl cmpb jbe|ja"; store="movq" ;;
  *) echo "unknown target $t"; fail=1; continue ;;
  esac
  read -r -a m <<<"$(echo "$mn" | tr '\n' ' ')"
  k=-1
  for i in "${!m[@]}"; do
    if [ "${m[$i]}" = "$first" ]; then k=$i; break; fi
  done
  ok=1
  if [ "$k" -lt 0 ] || [ "$k" -gt 3 ]; then
    echo "  cell test: $k instructions before the header load (want 1..3)"
    ok=0
  fi
  read -r -a want <<<"$seq"
  for j in 0 1 2 3; do
    got="${m[$((k + j))]:-}"
    if ! [[ "$got" =~ ^(${want[$j]})$ ]]; then
      echo "  instruction $((k + j)): got '$got', want '${want[$j]}'"
      ok=0
    fi
  done
  stores="$(printf '%s\n' "${m[@]:$((k + 4))}" | grep -cx "$store" || true)"
  if [ "$stores" -lt 1 ]; then
    echo "  no plain store ($store) after the header check"
    ok=0
  fi
  if [ $ok = 1 ]; then
    echo "  PASS: cell test $k + header check 4 (${m[*]:$k:4}) + store ($store)"
  else
    echo "  FAIL"
    fail=1
  fi
done
exit $fail
