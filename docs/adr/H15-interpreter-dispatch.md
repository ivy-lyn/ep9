# H15: Interpreter dispatch

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). Reopened by its own spike on x86-64 before the first draft; the x64 part is decided here. Accept needs the review record (README §3.1) |
| Fix by | M0 wk2 |
| Freeze point | M1 exit |
| Origin | v1#11; plan status E (extended 2026-10-21: release expansion per target) |
| Crates | `ep9_vm` (T0 interpreter: opcode DSL, handlers, dispatch table in `Cx`); `tools/xtask` (`disasm-check`) |
| Owner (plan role) | VM lead |
| Validated by | SP-Dispatch (M0 wk2): `spikes/sp-dispatch/gate.sh` and the CI spikes job on 4 targets |
| Plan sources | [02 §6 H15](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §2](../plan/04-execution.md#2-t0-techniques-that-reach-parity), [09 §4 SP-Dispatch](../plan/09-milestones.md#4-spikes), [10 §3 speed-gate rule](../plan/10-quality.md#3-benchmark-sets) |

## Context

- T0 runs every function first, and it is the deopt target and the jitless fallback (H5). Its dispatch cost is in every mode. G2-G4 need T0 parity with V8 Ignition (04 §2).
- Depends on D8 (stable toolchain; nightly `become` opt-in) and H26. H14 (frames) and D21 (Spike B) use a dispatch as their unit of cost.
- Risks: RK15 (stable sibling-call TCO is silently lost), RK25 (Win64 passes only 4 register arguments).
- SP-Dispatch CI run 1 ([39458801246](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246)) failed the gate "sibling ≥ 1.2x faster than match" on linux-x64 (0.812), and match beat sibling on every kernel there. That is the DISPATCH-GATE reopen trigger ([decisions-log §5.4](../decisions-log.md)). This draft records the reopen and its result.

## Decision

Macro DSL, 3 expansions (match, sibling-call table, nightly `become`). ≤ 4 handler args: Cx, pc, frame base, accumulator. An M5c generated interpreter may be AOT-emitted and embedded (02 §6).

- **One DSL.** `for_each_op!` lists each opcode once. Expansions: (a) `loop { match }`; (b) one handler per opcode that ends in `return table[op](cx, pc, fp, acc)`; (c) the same with nightly `become` (D8: opt-in).
- **Handler type.** `unsafe fn(&mut Cx, *const u8, *mut i64, i64) -> i64`. `Cx` holds the 256-entry dispatch table at offset 0. A trait bound rejects a 5th or a non-register-wide argument at compile time.
- **Cold blocks.** Every runtime call of a handler is in a cold block. The fast handler reaches it by a tail jump, so the fast path has no call and no frame.
- **Debug builds** use the match loop on every target. Debug sibling calls are real calls and overflow the stack.
- **Release expansion per target (2026-10-21).** It is a build choice of the DSL. The handler type, the `run` entry type and `Cx` are the same for both, so a change needs no other code change.

| Target | Release expansion | Reason (SP-Dispatch, [measurements](../measurements/2026-10-21-sp-dispatch-x64.md)) |
|---|---|---|
| aarch64 (macOS, Linux) | sibling calls (b) | The aarch64 match loop has 1 dispatch jump. Sibling is 1.37-2.05x faster than match (Apple M5 Max 1.37-1.49, macos-15 2.05, Neoverse-N2 1.71-1.77) and 0.91-1.02x cgoto.c (macos-15: 0.50, H15-Q4) |
| x86-64 SysV and Win64 | match loop (a) | LLVM copies the match dispatch into every arm (16 indirect jumps; 1 on aarch64), so the loop is already threaded code. Match / sibling time by CPU model (Linux): 0.77-0.81 (EPYC 7763, 7 jobs + CI), 0.88-0.92 (EPYC 9V74), 1.03 (EPYC 9V45), 1.13 (Xeon 8573C, Xeon 8370C). Windows: 0.83 (EPYC 7763), 0.97 (EPYC 9V74). Match is faster on AMD Zen 3 and Zen 4, within 3% on Zen 5, slower on both Intel models. No DSL change made sibling 1.2x faster than match on any AMD model |

- **No DSL codegen fix.** `jmp reg`, a shared JumpIfFalse jump, `cmov`, one taken jump per handler, `target-cpu=znver3` and 64-byte function alignment did not fix Zen 3. Alignment helps Zen 4 and Intel, but stable Rust can only align every function of the binary (LLVM flag). Not applied.
- **x64 threading is fragile.** The dispatch-counting copy of the loop adds one memory increment per dispatch, and LLVM then keeps 1 dispatch jump. So the x64 release loop has a blocking disassembly check (Rules).
- **x64 layout sensitivity.** On EPYC 7763 the loop kernel takes, in ns per dispatch on Linux (7 jobs) and on Windows (1 job): match 1.10-1.15 and 1.25, sibling 1.16-1.20 and 0.98, cgoto.c 0.98-1.09 (clang 18) and 0.80 (clang 20). Between the two builds sibling and cgoto.c move by 17-22%, match by 9-14%.

## Rules

| Rule | Check | From |
|---|---|---|
| All expansions and cgoto.c give equal results | `sp-dispatch check` against reference models; bench "results equal" | SP-Dispatch |
| ≤ 4 register-wide handler args, none in memory | `AtMostFourRegArgs` compile check; `disasm-check` LLVM IR check on 5 targets; 2 negative tests | SP-Dispatch; T0 from M1 |
| Every `tail` handler of (b) and (c) ends in a tail jump; 0 non-tail exits | `cargo xtask disasm-check` on 5 targets; deep loop on a 64 KiB stack; 2 negative tests | SP-Dispatch; T0 from M1 |
| The x86-64 release loop is threaded: dispatch jumps ≥ half of the fast tail handlers; no stack access except next to a call, in the entry block or in an exit sequence | `cargo xtask disasm-check --match-loop` (symbol `ep9loop__match__run`); a build without LLVM tail duplication must fail it | 2026-10-21; T0 from M1 |
| The release expansion is ≤ 1.25x cgoto.c time (geomean of loop, branchy, call) and ≤ 1.5x per kernel | `sp-dispatch bench` gate `<release>-vs-cgoto-<abi>` | SP-Dispatch |
| aarch64: match time ≥ 1.2x sibling time (geomean) and ≥ 1.0x per kernel | gate `sibling-vs-match-a64` | SP-Dispatch |
| x86-64: sibling vs match is measured and reported per CPU model, never gated | `REPORT sibling-vs-match` in bench output and `bench.json` | 2026-10-21 |
| Every speed result names its CPU model, rounds and verdict | bench `cpu:` line; `bench.json` | 2026-10-21 |

**Speed verdicts** follow the 10-quality §3 rule (PERF-NOISE): paired ratios per round, median, order-statistic interval (n = 15: ranks 4 and 12; n = 31: ranks 10 and 22). Pass or fail only if the whole interval is on one side, else marginal. Void if a kernel IQR > 5%.

| Target | Where a speed gate counts | Gates |
|---|---|---|
| aarch64 | the shared macOS arm64 host in a quiet window (10 §3, "until gate hosts exist"). Hosted arm64 results are provisional | `sibling-vs-cgoto-a64`, `sibling-vs-match-a64` |
| x86-64 SysV, Win64 | a hosted runner (class S allows it on x64) with ≥ 31 rounds; a verdict holds only for the named CPU model. For M0 exit item 5, no recorded model may fail; marginal and void runs are repeated | `match-vs-cgoto-x64-sysv`, `match-vs-cgoto-x64-win64` |

**CI rule.** Structural gates block: build, tests, agreement, deep loop, tail exits, handler args, x64 loop shape and the negative tests. Speed gates never block on hosted runners (`SP_SPEED=report`, 31 rounds). Each speed gate that does not pass gives a `::warning` annotation. Every gate goes to the job summary, and all samples go to the artifact `sp-dispatch-gate-<target>` (`bench.json`). Reason: the hosted x64 labels gave 5 CPU models in 16 jobs, and the x64 verdict changed with the model (0.77 to 1.13). A blocking gate would turn red or green by the draw of the runner.

## Consequences

- **Gains.** aarch64 keeps sibling dispatch at C computed-goto speed. x86-64 keeps the faster loop on EPYC Zen 3 (1.23-1.30x) and Zen 4 (1.08-1.13x); Zen 5 is within 3%. Both expansions stay built and checked on every target (debug, aarch64 and x64 releases).
- **Costs.** Two release shapes to tune in T0. On both Intel models (Xeon 8573C, 8370C) sibling is 1.13x faster than the x64 match loop. x64 speed depends on an LLVM heuristic (tail duplication). R06's "sibling 1.3-1.5x faster than match" holds on aarch64 only.
- **Follow-ups.** T0 (M1) keeps the `--match-loop` check on the real handler set. D21 and Spike B: "18 sibling-call dispatches" means release-expansion dispatches of the loop kernel on the same host (decisions-log §7). 10 §3 class S needs the CPU-model clause (decisions-log §7).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | Tail-call dispatch 1.3-1.5x faster than match; measured on Apple M only | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [P1] | A 5th argument breaks every tail call on Win64 (SP-Dispatch negative test) | [../../spikes/sp-dispatch/README.md](../../spikes/sp-dispatch/README.md) |
| SP-Dispatch x64 | x64 match loop threaded by LLVM; per-CPU ratios; cachegrind: x64 match 1.05-1.23x the instructions of sibling, aarch64 1.34-1.64x | [../measurements/2026-10-21-sp-dispatch-x64.md](../measurements/2026-10-21-sp-dispatch-x64.md) |

## Validation

- SP-Dispatch (M0 wk2): `bash spikes/sp-dispatch/gate.sh` (host) and the CI spikes job (4 targets). Diag runs: [39464428627](https://github.com/Brooooooklyn/ep9/actions/runs/39464428627), [39466346055](https://github.com/Brooooooklyn/ep9/actions/runs/39466346055), [39468143183](https://github.com/Brooooooklyn/ep9/actions/runs/39468143183), Windows [39463155980](https://github.com/Brooooooklyn/ep9/actions/runs/39463155980). New gate and CI rule on branch `diag/h15-gate`: runs [39472609604](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604) and [39474039711](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711). All 4 jobs pass both times; structural gates pass on 4 targets; the x64 negative build fails as expected. `match-vs-cgoto-x64-sysv` passes on Xeon 8370C (1.128, twice). `match-vs-cgoto-x64-win64`: void both times; interval fail on EPYC 7763 (loop kernel 1.591), pass on EPYC 9V74 (H15-Q3). aarch64 speed gates: interval pass in all 4 jobs (2 pass, 6 void).
- Local gate 2026-10-21 (Apple M5 Max, not a quiet window): all structural gates pass on aarch64 and statically on x64 SysV and Win64; `sibling-vs-cgoto-a64` pass 0.975; `sibling-vs-match-a64` 1.489, void (call IQR 5.0%).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H15-Q1 | x64 release expansion with the T0 opcode set: match, sibling, sibling + alignment, or a pick at startup by CPU model | M2 exit (T0 parity), on ≥ 3 named x64 models incl. one Intel client | match (this ADR) |
| H15-Q2 | Why sibling code pays about 5x more per mispredicted branch on Zen 3 | before H15-Q1 | `perf stat` on a bare-metal Zen 3 host (hosted AMD VMs have no PMU) |
| H15-Q3 | The cgoto.c compiler is not pinned (clang 18 Linux, 20 Windows, Apple clang 17 macos-15). Win64 on EPYC 7763: loop kernel 1.591x cgoto.c (interval fail, run void), mostly because cgoto.c and sibling run that kernel faster there than on Linux | M0 exit item 5 | Pin one LLVM version for cgoto.c on all targets; repeat Win64 on EPYC 7763. If the guard still fails, Win64 release moves to sibling (sibling / cgoto.c there: 1.233, max kernel 1.381) |
| H15-Q4 | macos-15 (Apple M2 Pro, virtual) cgoto.c is 2x slower than sibling. It uses Apple clang 17.0.0; Apple clang 21 and Homebrew clang 23 give the same code and speed locally | M0 exit | covered by the H15-Q3 compiler pin |

## Reopens if

- The aarch64 speed gates fail on the gate host in a quiet window (09 §4).
- The x64 loop check fails on T0: x64 release moves to sibling calls, and this ADR reopens.
- On a named x64 model, `match-vs-cgoto` fails (a PERF-NOISE fail with ≥ 31 rounds).
- Not a trigger: sibling beating match on a named x64 model. That split is known (AMD Zen 3/4: match faster; Intel: sibling faster). It is H15-Q1 input.
- A handler needs a 5th argument. D8 changes (`become` on stable).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | reopen + draft | First draft. DISPATCH-GATE trigger fired on x64 (CI run 39458801246: 0.812; match beats sibling on a kernel). Release expansion per target (x64: match); gate per target; PERF-NOISE verdicts in the bench; CI speed gates non-blocking with warnings, summary and artifact | H15 agent (M0 workflow) |
