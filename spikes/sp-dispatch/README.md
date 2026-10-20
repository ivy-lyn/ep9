# SP-Dispatch (M0 week 2, throwaway)

This spike tests H15: one opcode DSL, three dispatch expansions, at most 4 handler arguments.
Gate source: [docs/plan/09-milestones.md](../../docs/plan/09-milestones.md) section 4. Decision record:
[H15 ADR](../../docs/adr/H15-interpreter-dispatch.md). Evidence: [R06], [P1],
[x64 measurements](../../docs/measurements/2026-10-21-sp-dispatch-x64.md).

**Release expansion per target (H15, 2026-10-21):** sibling calls on aarch64; the match loop on
x86-64, where LLVM copies the dispatch jump into every match arm (threaded code). `Variant::RELEASE`
in `src/vm.rs` holds the choice. The first CI run failed "sibling >= 1.2x faster than match" on
linux-x64 (0.812); that reopened H15 (DISPATCH-GATE rule).

| Gate id | Kind | Rule | Result |
|---|---|---|---|
| dsl-3-expansions | structural | 20 opcodes defined once; match, sibling call and `become` build and agree | pass (4 CI targets) |
| tail-exits-a64, -x64-sysv, -x64-win64 | structural | every `tail` handler of sibling and `become` ends in a tail jump; 0 non-tail exits | pass (binaries; staticlib objects on 5 targets) |
| handler-args | structural | <= 4 handler arguments, none passed in memory | pass (compile-time trait check + LLVM IR on 5 targets) |
| match-loop-x64-sysv, -x64-win64 | structural | the x86-64 match loop has a dispatch jump for >= half of the 19 fast tail handlers and no stack access away from a call, the entry or an exit; a build without tail duplication must fail | pass: 16 jumps; negative fails as expected |
| sibling-vs-cgoto-a64 | speed | sibling time <= 1.25x cgoto.c (geomean of 3 kernels), <= 1.5x per kernel | pass: 0.975 (2026-10-21, provisional host) |
| sibling-vs-match-a64 | speed | match time >= 1.2x sibling time (geomean), >= 1.0x per kernel | 1.489, void (call IQR 5.0%); 2026-10-20: 1.365 |
| match-vs-cgoto-x64-sysv, -x64-win64 | speed | match time <= 1.25x cgoto.c (geomean), <= 1.5x per kernel | sysv: pass 1.128 (Xeon 8370C, twice); win64: void, loop 1.591 on EPYC 7763 (H15-Q3) |

- Speed verdicts follow docs/plan/10-quality.md section 3 (PERF-NOISE): paired ratios per round,
  median, order-statistic interval (pass or fail only if the whole interval is on one side, else
  marginal), void if a kernel IQR > 5%. On x86-64 bench also prints `REPORT sibling-vs-match`
  (not a gate there).
- CI runs `SP_SPEED=report` with 31 rounds: structural gates block; speed gates give warnings, a
  job summary and the artifact `sp-dispatch-gate-<target>` (`bench.json` with every sample).

Host of the 2026-10-20 run: Apple M5 Max (6 P + 12 E cores), load average 16-21. rustc 1.99.1,
nightly-2026-10-04 for `become`, Homebrew clang 23.1.3 `-O2` for cgoto.c.

## Layout

| File | Content |
|---|---|
| `src/ops.rs` | `for_each_op!`: the 20 opcodes, written once |
| `src/dsl.rs` | Expansion macros, operand decoders, handler signature check |
| `src/vm.rs` | `Cx` (dispatch table at offset 0), `Handler`, the three expansions, runner |
| `src/program.rs` | Bytecode container, `.ep9d` file format (shared with C), verifier |
| `src/asm.rs`, `src/kernels.rs` | Assembler; 3 benchmark kernels with plain-Rust reference models; test programs |
| `src/main.rs` | CLI: `run`, `dump`, `ops`, `check`, `deep-loop`, `cpu`, `bench` (speed gates, `--json`) |
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
  (cold handlers: `<op>_cold`). The match loop is `ep9loop__match__run` (and `__run_counting`).
  `cargo xtask disasm-check` finds them by these names. rustc 1.99.1 uses v0 mangling, so a
  mangled-name pattern would be fragile. The export changes one entry instruction of the loop
  (`acc` from the argument instead of a constant 0); the arms are byte-identical.

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

### x86-64 (2026-10-21)

Hosted runners, 15 rounds. Full tables, probes and run URLs: [x64 measurements](../../docs/measurements/2026-10-21-sp-dispatch-x64.md).

| CPU (`ubuntu-24.04`) | Jobs | match / sibling time (geomean) | match / cgoto.c time |
|---|---|---|---|
| AMD EPYC 7763 (Zen 3) | 7 + CI run 1 | 0.771-0.813 (branchy 0.556-0.586) | 0.87-0.96 |
| AMD EPYC 9V74 (Zen 4) | 3 | 0.883-0.923 | 1.056-1.066 |
| AMD EPYC 9V45 (Zen 5) | 1 | 1.025 | 1.008 |
| Intel Xeon Platinum 8573C | 1 | 1.132 | 1.189 |
| Intel Xeon Platinum 8370C | 2 (new gate, 31 rounds) | 1.126-1.128 | 1.128 |
| Neoverse-N2 (`ubuntu-24.04-arm`) | 3 + CI run 1 | 1.712-1.765 | 1.60-1.75 |

Windows (`windows-2025`): match / sibling 1.099 and 0.933 (CPU not printed); 0.834 on EPYC 7763 and
0.971 on EPYC 9V74 (31 rounds). Match / cgoto.c on the loop kernel: 1.384, 1.664, 1.591, 1.328. On EPYC
7763, cgoto.c (clang 20) and sibling run the loop kernel 17-22% faster on Windows than on Linux (H15-Q3).

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

0. (2026-10-21) On x86-64 LLVM copies the match dispatch into every arm (16 indirect jumps;
   1 on aarch64). So the x64 match loop is already threaded, and sibling calls gain nothing
   there: match / sibling time is 0.77-1.13 by CPU model. The x86-64 release expansion is the
   match loop; its shape is fragile (the dispatch-counting copy has 1 jump), so a blocking
   disassembly check guards it.

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

- Linux and Windows timings come from hosted CI runners only (class S, CPU model named from
  2026-10-21). No bare-metal x64 gate host; no Intel or AMD client CPU.
- `become` was not built for x86_64-apple-darwin (no nightly std for it here).
- cgoto.c trusts the bytecode file (the Rust side verifies before it writes it).

## Commands

```bash
bash spikes/sp-dispatch/gate.sh          # host gate; logs and bench.json in spikes/sp-dispatch/target/gate/
SP_SPEED=report SP_ROUNDS=31 bash spikes/sp-dispatch/gate.sh   # as CI: speed gates recorded, not in the exit code
cargo xtask disasm-check --all-targets --expect 26 --match-loop  # static checks, 5 targets
cargo xtask disasm-check --toolchain nightly-2026-10-04 --features become --expect 26
spikes/sp-dispatch/target/release/sp-dispatch bench --rounds 15 --cgoto spikes/sp-dispatch/target/cgoto
```

gate.sh environment: `SP_ROUNDS` (15), `SP_SPEED` (`gate`; `report` keeps speed verdicts out of
the exit code), `SP_NIGHTLY` (nightly-2026-10-04; `none` skips `become`), `CC` (clang),
`SP_CROSS=0` skips the cross-target objects, `SP_BENCH=0` skips timing.
