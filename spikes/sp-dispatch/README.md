# SP-Dispatch (M0 week 2, throwaway)

This spike tests H15: one opcode DSL, three dispatch expansions, at most 4 handler arguments.
Gate source: [docs/plan/09-milestones.md](../../docs/plan/09-milestones.md) section 4. Evidence: [R06], [P1].

| Gate id | Rule | Result on 2026-10-20 |
|---|---|---|
| dsl-3-expansions | 20 opcodes defined once; match, sibling call and `become` build and agree | pass |
| sibling-vs-cgoto-a64 | sibling time <= 1.25x cgoto.c (geomean of 3 kernels) | pass: 0.998 |
| sibling-vs-match-a64 | sibling >= 1.2x faster than match (geomean) | pass: 1.365 |
| tail-exits-a64 | every `tail` handler ends in a tail jump; 0 non-tail exits | pass (macOS binary + staticlib; Linux object) |
| tail-exits-x64-sysv | same, x86-64 SysV | pass (Linux object; macOS object and binary) |
| tail-exits-win64 | same, x86_64-pc-windows-msvc | pass (object only; no Windows run) |
| handler-args | <= 4 handler arguments, none passed in memory | pass (compile-time trait check + LLVM IR on 5 targets) |

Host: Apple M5 Max (6 P + 12 E cores), load average 16-21 during the gate run. rustc 1.99.1,
nightly-2026-10-04 for `become`, Homebrew clang 23.1.3 `-O2` for cgoto.c.

## Layout

| File | Content |
|---|---|
| `src/ops.rs` | `for_each_op!`: the 20 opcodes, written once |
| `src/dsl.rs` | Expansion macros, operand decoders, handler signature check |
| `src/vm.rs` | `Cx` (dispatch table at offset 0), `Handler`, the three expansions, runner |
| `src/program.rs` | Bytecode container, `.ep9d` file format (shared with C), verifier |
| `src/asm.rs`, `src/kernels.rs` | Assembler; 3 benchmark kernels with plain-Rust reference models; test programs |
| `src/main.rs` | CLI: `run`, `dump`, `ops`, `check`, `deep-loop`, `bench` |
| `cgoto.c` | The same 20 opcodes as a C computed-goto interpreter; reads the same `.ep9d` files |
| `tests/vm.rs` | Variants vs reference models, cold paths, stack overflow, verifier rejections |
| `gate.sh` | Host gate (see below) |
| `../../tools/xtask/src/disasm.rs` | `cargo xtask disasm-check` |

## The DSL

```text
ops.rs: for_each_op!(M [args])  --->  M! { state(cx, pc, next_pc, fp, acc); <20 entries> }

entry:  class snake Camel = opcode (operand: Kind, ..) => { fast body } [cold { cold body }];
        class = tail (always continues) | exit (may return to the host)
        body returns Flow::{Next, Jump(pc), Exit(v), Cold}

expand_meta!      Op enum, OPS table (assembler, verifier)
expand_match!     (a) loop { match op { .. } }       cold block inline in its arm
expand_threaded!  (b) return table[op](cx, pc, fp, acc)   one fn per opcode + one per cold block
                  (c) become table[op](cx, pc, fp, acc)   nightly, feature `become`
```

- **Handler type (H15).** `unsafe fn(&mut Cx, pc: *const u8, fp: *mut i64, acc: i64) -> i64`.
  `Cx` holds the 256-entry dispatch table at offset 0. A trait bound (`AtMostFourRegArgs`)
  rejects a 5th argument or an argument that is not register wide, at compile time.
- **Cold blocks.** A fast handler reaches its cold handler by a tail jump. So the fast path
  has no call and needs no stack frame. All runtime calls (int32 overflow, interrupt check,
  stack overflow) are in cold blocks.
- **Symbol contract.** Each handler is exported as `ep9h__<expansion>__<tail|exit>__<op>`
  (cold handlers: `<op>_cold`). `cargo xtask disasm-check` finds handlers by this name.
  rustc 1.99.1 uses v0 mangling, so a mangled-name pattern would be fragile.

## Bytecode

Register + accumulator, 1-byte opcodes, values are `i64` with an int32 fast path.

| # | Op | Operands | # | Op | Operands |
|---|---|---|---|---|---|
| 0 | Halt (exit) | - | 10 | BitXor | reg |
| 1 | LdaSmi | imm8 | 11 | ShlSmi | imm8 |
| 2 | LdaConst | const | 12 | ShrSmi | imm8 |
| 3 | Ldar | reg | 13 | TestLessThan | reg |
| 4 | Star | reg | 14 | TestEqual | reg |
| 5 | Mov | reg, reg | 15 | Jump | off16 |
| 6 | Add (cold) | reg | 16 | JumpIfFalse | off16 |
| 7 | Sub (cold) | reg | 17 | JumpLoop (cold: interrupt) | off16 |
| 8 | AddSmi (cold) | imm8 | 18 | Call (cold: stack overflow) | func, reg, count |
| 9 | BitAnd | reg | 19 | Return | - |

Call pushes a VM frame (header: back offset, return pc offset, register count) and never
recurses on the native stack. The verifier checks every operand, so handlers read without
bounds checks.

| Kernel | Work | Dispatches |
|---|---|---|
| loop | n = 40,000,000: bit ops, int32 add/sub, compare, back edge | 880,000,017 |
| branchy | n = 15,000,000: xorshift32, two data-dependent branches | 498,736,356 |
| call | fib(35): one VM call per about 9.5 dispatches | 283,676,681 |

## Results

### aarch64-apple-darwin (the gate host)

Median of 15 runs, one process per run, variant order rotated every round, warm-up round
dropped. The timer covers the interpreter run only. All variants gave equal results.

| Kernel | match ms (ns/dispatch) | sibling | become | cgoto.c | sibling / cgoto | match / sibling |
|---|---|---|---|---|---|---|
| loop | 510.4 (0.580) | 312.8 (0.355) | 313.2 (0.356) | 314.9 (0.358) | 0.993 | 1.632 |
| branchy | 337.1 (0.676) | 285.3 (0.572) | 284.3 (0.570) | 283.7 (0.569) | 1.006 | 1.181 |
| call | 195.8 (0.690) | 148.6 (0.524) | 141.2 (0.498) | 149.3 (0.526) | 0.995 | 1.318 |
| geomean | | | | | **0.998** | **1.365** |

- `become` / sibling: 0.982 (geomean); 0.982-1.006 over three runs.
- branchy alone is below 1.2x (1.181 here; 1.155 and 1.258 in two earlier runs). Branch
  misses dominate it, so dispatch cost matters less.
- Cold-block A/B (11 rounds, feature `cold-inline` = runtime calls in the fast handlers):
  cold-inline / sibling = 0.988. On a64 the cold blocks give no measured speed gain. Their
  value is on x64 (next table).

### Disassembly, sibling expansion (26 handlers: 20 + illegal + 5 cold)

| Target | Object | Tail handlers ending in tail jumps | Non-tail exits | Fast handlers with a frame on entry |
|---|---|---|---|---|
| aarch64-apple-darwin | binary, staticlib | 24/24 | 0 | none (before cold blocks: call, jump_loop) |
| aarch64-unknown-linux-gnu | staticlib object | 24/24 | 0 | none |
| x86_64-unknown-linux-gnu | staticlib object | 24/24 | 0 | call (before: add, add_smi, sub, jump_loop, call) |
| x86_64-pc-windows-msvc | staticlib object | 24/24 | 0 | call (before: add, add_smi, sub, jump_loop, call) |
| x86_64-apple-darwin | binary, staticlib | 24/24 | 0 | all 26 (`push rbp`: Apple x86-64 keeps frame pointers) |

- `become` (nightly) on aarch64-apple-darwin and x86_64-unknown-linux-gnu: 24/24, 0 non-tail exits.
- x64 frames (SysV and Win64): LLVM did not shrink-wrap the handlers with a runtime call, so
  each execution paid a prologue and an epilogue, fast path included. On a64 it did for
  add/sub. Cold blocks remove the call, and with it the frame.
- `call` keeps a frame on x64: LLVM vectorizes the argument copy and needs callee-saved registers.

### Negative tests (the checks must fail)

| Build | Check | Result |
|---|---|---|
| `neg-escape`: `mov` passes `&mut acc` to an opaque call | disasm-check | FAIL: `blr` + `ret` in `mov` |
| `neg-escape` | deep-loop on a 64 KiB native stack | aborts: "has overflowed its stack" |
| `neg-five-args` | compile | error E0277: `AtMostFourRegArgs` not satisfied |
| `neg-five-args,neg-skip-arg-check` | disasm-check, 5 targets | IR args FAIL (max 5) on all 5 targets |
| same | tail jumps per target | a64 24/24, SysV 24/24, **Win64 0/24** (57 non-tail exits) |

The Win64 5-argument handler is `callq *(%rcx,%r11,8)` + `retq`: the 5th argument lives in a
stack slot, so the dispatch becomes a real call. This re-checks [P1] (RK25).

### Debug builds

- Debug sibling calls are real calls: the runaway test (9,363 dispatches, one native frame
  each) overflows the 2 MiB test thread stack. So debug builds must use the match loop (plan rule).
- Debug `become` runs the same test without growth (guaranteed tail call, D8 note).

### x86_64-apple-darwin under Rosetta (not valid for the gate)

11 runs: sibling / cgoto = 1.048, match / sibling = 1.019 (geomean). The dispatch gain
mostly disappears. A probable cause (not verified): Rosetta handles each x86 indirect jump
with its own target lookup.

## Findings for H15

1. Stable sibling calls reach C computed-goto speed on a64 (0.998x) and beat the match loop
   by 1.365x. Nightly `become` is within 2% of them. Stable is enough for T0.
2. The 4-argument rule is real on Win64 only: a 5th argument breaks every tail call there.
3. Put every runtime call of a handler in a cold block (tail jump to a cold handler). Else
   x64 and Win64 pay a stack frame on the fast path.
4. The disassembly check must use relocations (relocatable ELF/COFF show call targets as
   the next instruction) and handle merged functions (LLVM aliases identical handlers).
5. A deep-loop test only catches handlers that run many times. It needs a kernel that runs
   every opcode in a loop (`all-ops`).

## Not done or not measured

- No timing on Linux or Windows hardware; those gates (speed per target) need CI runners.
- `become` was not built for x86_64-apple-darwin (no nightly std for it here).
- cgoto.c trusts the bytecode file (the Rust side verifies before it writes it).

## Commands

```bash
bash spikes/sp-dispatch/gate.sh          # host gate; logs in spikes/sp-dispatch/target/gate/
cargo xtask disasm-check --all-targets --expect 26     # static checks, 5 targets
cargo xtask disasm-check --toolchain nightly-2026-10-04 --features become --expect 26
spikes/sp-dispatch/target/release/sp-dispatch bench --rounds 15 --cgoto spikes/sp-dispatch/target/cgoto
```

gate.sh environment: `SP_ROUNDS` (15), `SP_NIGHTLY` (nightly-2026-10-04; `none` skips
`become`), `CC` (clang), `SP_CROSS=0` skips the cross-target objects, `SP_BENCH=0` skips timing.
