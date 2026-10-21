# H15: Interpreter dispatch

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). Reopened twice on x86-64 the same day: by its spike (x64 release: match loop), then by the T0 trigger "the x64 loop check fails on T0" (x64 release: sibling calls, this revision). Accept needs the review record (README §3.1) |
| Fix by | M0 wk2 |
| Freeze point | M1 exit |
| Origin | v1#11; plan status E (extended 2026-10-21: release expansion per target) |
| Crates | `ep9_vm` (T0 interpreter: opcode DSL, handlers, dispatch table in `Cx`); `tools/xtask` (`disasm-check`) |
| Owner (plan role) | VM lead |
| Validated by | SP-Dispatch (M0 wk2): `spikes/sp-dispatch/gate.sh` and the CI spikes job on 4 targets. T0 (M0 wk3): `cargo xtask disasm-check` on `ep9_vm`, `tools/bench/workloads/t0-dispatch.mjs` on hosted runners ([T0 measurements](../measurements/2026-10-21-t0-dispatch-x64.md)) |
| Plan sources | [02 §6 H15](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §2](../plan/04-execution.md#2-t0-techniques-that-reach-parity), [09 §4 SP-Dispatch](../plan/09-milestones.md#4-spikes), [10 §3 speed-gate rule](../plan/10-quality.md#3-benchmark-sets) |

## Context

- T0 runs every function first, and it is the deopt target and the jitless fallback (H5). Its dispatch cost is in every mode. G2-G4 need T0 parity with V8 Ignition (04 §2).
- Depends on D8 (stable toolchain; nightly `become` opt-in) and H26. H14 (frames) and D21 (Spike B) use a dispatch as their unit of cost.
- Risks: RK15 (stable sibling-call TCO is silently lost), RK25 (Win64 passes only 4 register arguments).
- SP-Dispatch CI run 1 ([39458801246](https://github.com/Brooooooklyn/ep9/actions/runs/39458801246)) failed the gate "sibling ≥ 1.2x faster than match" on linux-x64 (0.812), and match beat sibling on every kernel there. That is the DISPATCH-GATE reopen trigger ([decisions-log §5.4](../decisions-log.md)). The first draft recorded that reopen: x86-64 release = match loop, because LLVM threaded the spike's match loop.
- T0 reopen (2026-10-21, later the same day). The trigger "the x64 loop check fails on T0" fired on `ep9_vm` (157 opcodes, 282 handlers): `disasm-check --match-loop` finds 2 indirect jumps (≥ 79 needed) and 92-134 stack accesses away from calls on x86_64 Linux, Windows and macOS. The T0 match loop is a switch loop with 1 dispatch jump, not threaded code. This revision decides the x86-64 release expansion again, with T0 data ([T0 measurements](../measurements/2026-10-21-t0-dispatch-x64.md)).

## Decision

Macro DSL, 3 expansions (match, sibling-call table, nightly `become`). ≤ 4 handler args: Cx, pc, frame base, accumulator. An M5c generated interpreter may be AOT-emitted and embedded (02 §6).

- **One DSL.** `for_each_op!` lists each opcode once. Expansions: (a) `loop { match }`; (b) one handler per opcode that ends in `return table[op](cx, pc, fp, acc)`; (c) the same with nightly `become` (D8: opt-in).
- **Handler type.** `unsafe fn(&mut Cx, *const u8, *mut i64, i64) -> i64`. `Cx` holds the 256-entry dispatch table at offset 0. A trait bound rejects a 5th or a non-register-wide argument at compile time.
- **Handler ABI (T0, 2026-10-21).** On x86-64 the handlers are `extern "sysv64-unwind"` on every OS (`dsl::handler!`), and plain `extern "sysv64"` under `panic = "abort"` (the release profile, H23; M2-P2-ABORT-SYSV64): from abort code, a call to an `-unwind` function gets an abort landing pad, which turns the sibling calls into call + return (`disasm-check`: 50 of 282 tail jumps), and nothing can unwind there anyway. Win64 handlers then get the SysV register set: `rsi`, `rdi` and `xmm6-xmm15` are not callee-saved. Fast handlers that build a frame on Win64: 34 → 12 (Linux: 11). Windows sibling time, old / new (geomean of 4 T0 kernels): 1.13 (EPYC 7763; `branchy` 1.40-1.44), 1.04 (Xeon 8573C), 1.02 (EPYC 9V45). On Linux and macOS the convention does not change (Linux control: 0.996, 1.003). Other targets use the Rust ABI. The ≤ 4 argument rule stays (aarch64 and the IR check).
- **Cold blocks.** Every runtime call of a handler is in a cold block. The fast handler reaches it by a tail jump, so the fast path has no call and no frame.
- **Debug builds** use the match loop on every target. Debug sibling calls are real calls and overflow the stack.
- **Release expansion: sibling calls (b) on every target (T0, 2026-10-21).** `Dispatch::RELEASE` in `crates/ep9_vm/src/interp/mod.rs`. It is a build choice of the DSL: the handler type, the `run` entry type and `Cx` are the same for every expansion. Debug builds keep the match loop.

| Target | Release expansion | Reason |
|---|---|---|
| aarch64 (macOS, Linux) | sibling calls (b) | Unchanged. T0: the match loop has 1 dispatch jump; match / sibling time (geomean of 4 T0 kernels) 1.52-1.53 (Apple M2 Pro, virtual), 1.26 (Neoverse-N2), 1.22 (Apple M5 Max, provisional). SP-Dispatch: 1.37-2.05 |
| x86-64 SysV and Win64 | sibling calls (b), since the T0 reopen | The T0 match loop is not threaded (1 dispatch jump; the reachable `_ => bad_opcode` arm keeps a constant-false range check that splits the dispatch block, and the block has too many predecessors and successors for early tail duplication). Match / sibling time, geomean of 4 T0 kernels, 22 hosted jobs: EPYC 7763 1.42-1.44 (Linux), 1.21-1.22 (Windows); EPYC 9V74 1.44-1.47, 1.22-1.25 (Windows); EPYC 9V45 1.26-1.28; Xeon 8370C 1.42-1.43; Xeon 8573C 1.30 (Windows). Every kernel on every job: interval wholly above 1. Instructions (cachegrind): match runs 1.09-1.63x those of sibling |

- **SP-Dispatch history (superseded on x86-64).** The spike made the x64 match loop the release expansion: its 26-arm loop had `_ => unreachable_unchecked()`, and LLVM copied the dispatch into the arms (16 indirect jumps). There, match / sibling time was 0.77-0.81 (EPYC 7763), 0.88-0.92 (9V74), 1.03 (9V45), 1.13 (Xeon 8573C, 8370C). That comparison was threaded code against threaded code. The T0 loop is not threaded, and the spike's AMD advantage of match does not appear in T0 (EPYC 7763 `branchy`: spike 0.56-0.59, T0 1.66-1.72).
- **No DSL codegen fix (spike).** `jmp reg`, a shared JumpIfFalse jump, `cmov`, one taken jump per handler, `target-cpu=znver3` and 64-byte function alignment did not fix the spike's Zen 3 result. Alignment helps Zen 4 and Intel, but stable Rust can only align every function of the binary (LLVM flag). Not applied.
- **x64 threading is fragile, and T0 lost it.** The spike's counting copy lost the threading with one memory increment. T0 lost it with a reachable default arm and 157 arms. A threaded T0 loop needs `unreachable_unchecked()` for bad opcodes (undefined behavior on a VM bug) and `-Cllvm-args=-tail-dup-pred-size=1000 -tail-dup-succ-size=1000` for the whole binary; it then has 251 dispatch jumps but 320 stack accesses away from calls. Not applied.
- **x64 layout sensitivity.** On EPYC 7763 the loop kernel takes, in ns per dispatch on Linux (7 jobs) and on Windows (1 job): match 1.10-1.15 and 1.25, sibling 1.16-1.20 and 0.98, cgoto.c 0.98-1.09 (clang 18) and 0.80 (clang 20). Between the two builds sibling and cgoto.c move by 17-22%, match by 9-14%.

## Rules

| Rule | Check | From |
|---|---|---|
| All expansions and cgoto.c give equal results | `sp-dispatch check` against reference models; bench "results equal" | SP-Dispatch |
| ≤ 4 register-wide handler args, none in memory | `AtMostFourRegArgs` compile check; `disasm-check` LLVM IR check on 5 targets; 2 negative tests | SP-Dispatch; T0 from M1 |
| Every `tail` handler of (b) and (c) ends in a tail jump; 0 non-tail exits | `cargo xtask disasm-check` on 5 targets; deep loop on a 64 KiB stack; 2 negative tests | SP-Dispatch; T0 from M1 |
| T0 (`ep9_vm`): 284 handlers; every `tail` handler ends in a tail jump; 0 non-tail exits; ≤ 4 handler args | `cargo xtask disasm-check --manifest-path crates/ep9_vm/Cargo.toml --all-targets --expect 284` (5 targets); Mach-O data in code is not code | T0, 2026-10-21 |
| The match loop is the debug expansion only. Its release shape is a report: `--match-loop` prints it; the threaded-loop rule applies to SP-Dispatch, not to T0 | `cargo xtask disasm-check --match-loop` (symbol `ep9loop__match__run`) | T0, 2026-10-21 (was: the x86-64 release loop is threaded) |
| The release expansion is ≤ 1.25x cgoto.c time (geomean of loop, branchy, call) and ≤ 1.5x per kernel | `sp-dispatch bench` gate `<release>-vs-cgoto-<abi>` | SP-Dispatch |
| aarch64: match time ≥ 1.2x sibling time (geomean) and ≥ 1.0x per kernel | gate `sibling-vs-match-a64` | SP-Dispatch |
| x86-64: sibling vs match is measured and reported per CPU model, never gated | SP-Dispatch: `REPORT sibling-vs-match` in bench output and `bench.json`. T0: `node tools/bench/workloads/t0-dispatch.mjs` (31 paired rounds; JSON per job) | 2026-10-21 |
| Every speed result names its CPU model, rounds and verdict | bench `cpu:` line; `bench.json` | 2026-10-21 |

**Speed verdicts** follow the 10-quality §3 rule (PERF-NOISE): paired ratios per round, median, order-statistic interval (n = 15: ranks 4 and 12; n = 31: ranks 10 and 22). Pass or fail only if the whole interval is on one side, else marginal. Void if a kernel IQR > 5%.

| Target | Where a speed gate counts | Gates |
|---|---|---|
| aarch64 | the shared macOS arm64 host in a quiet window (10 §3, "until gate hosts exist"). Hosted arm64 results are provisional | `sibling-vs-cgoto-a64`, `sibling-vs-match-a64` |
| x86-64 SysV, Win64 | a hosted runner (class S allows it on x64) with ≥ 31 rounds; a verdict holds only for the named CPU model. For M0 exit item 5, no recorded model may fail; marginal and void runs are repeated | `sibling-vs-cgoto-x64-sysv`, `sibling-vs-cgoto-x64-win64` (since the T0 revision; `match-vs-cgoto` and `sibling-vs-match` are reports) |

**CI rule.** Structural gates block: build, tests, agreement, deep loop, tail exits, handler args, x64 loop shape and the negative tests. Speed gates never block on hosted runners (`SP_SPEED=report`, 31 rounds). Each speed gate that does not pass gives a `::warning` annotation. Every gate goes to the job summary, and all samples go to the artifact `sp-dispatch-gate-<target>` (`bench.json`). Reason: the hosted x64 labels gave 5 CPU models in 16 jobs, and the x64 verdict changed with the model (0.77 to 1.13). A blocking gate would turn red or green by the draw of the runner.

## Consequences

- **Gains.** One release shape on every target: T0 tuning, the deopt target and D21 dispatch counts use one expansion. On x86-64 T0 is 1.21-1.47x faster than with the match loop (geomean, 5 models), and no x64 speed depends on LLVM tail duplication. Both expansions stay built and checked (debug: match; release: sibling).
- **Costs.** x86-64 speed now depends on sibling calls staying tail jumps (RK15); `disasm-check` checks it on 5 targets. The spike's Zen 3 result (sibling pays about 5x per mispredicted branch against a threaded match loop, H15-Q2) does not apply to T0, but returns if a threaded match loop ever comes back. Win64 handlers use a non-default ABI (`sysv64-unwind`; `sysv64` under `panic = "abort"`).
- **Follow-ups.** Done on 2026-10-21 ([decisions-log §10](../decisions-log.md#10-2026-10-21-pr-1-review-fixes-ci-gates-and-h15-on-t0)): the CI `disasm` job runs the T0 row of Rules on the host target of 4 runners; SP-Dispatch `gate.sh` and bench gate sibling vs cgoto.c on x86-64 (`match-vs-cgoto` is a report); the plan rows (02 §6 H15, 04 §2, 09 §4 SP-Dispatch) and decisions-log §7 state the T0 result; the 10 §3 class S row names the CPU model. D21 and Spike B: "18 sibling-call dispatches" means release-expansion dispatches (now sibling everywhere).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | Tail-call dispatch 1.3-1.5x faster than match; measured on Apple M only | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [P1] | A 5th argument breaks every tail call on Win64 (SP-Dispatch negative test) | [../../spikes/sp-dispatch/README.md](../../spikes/sp-dispatch/README.md) |
| SP-Dispatch x64 | x64 match loop threaded by LLVM; per-CPU ratios; cachegrind: x64 match 1.05-1.23x the instructions of sibling, aarch64 1.34-1.64x | [../measurements/2026-10-21-sp-dispatch-x64.md](../measurements/2026-10-21-sp-dispatch-x64.md) |
| T0 x64 | T0 match loop not threaded (2 indirect jumps); sibling faster on 5 x64 and 2 arm64 models, every kernel; cachegrind: match 1.09-1.63x the instructions of sibling; Win64 frames with the SysV handler ABI | [../measurements/2026-10-21-t0-dispatch-x64.md](../measurements/2026-10-21-t0-dispatch-x64.md) |

## Validation

- SP-Dispatch (M0 wk2): `bash spikes/sp-dispatch/gate.sh` (host) and the CI spikes job (4 targets). Diag runs: [39464428627](https://github.com/Brooooooklyn/ep9/actions/runs/39464428627), [39466346055](https://github.com/Brooooooklyn/ep9/actions/runs/39466346055), [39468143183](https://github.com/Brooooooklyn/ep9/actions/runs/39468143183), Windows [39463155980](https://github.com/Brooooooklyn/ep9/actions/runs/39463155980). New gate and CI rule on branch `diag/h15-gate`: runs [39472609604](https://github.com/Brooooooklyn/ep9/actions/runs/39472609604) and [39474039711](https://github.com/Brooooooklyn/ep9/actions/runs/39474039711). All 4 jobs pass both times; structural gates pass on 4 targets; the x64 negative build fails as expected. `match-vs-cgoto-x64-sysv` passes on Xeon 8370C (1.128, twice). `match-vs-cgoto-x64-win64`: void both times; interval fail on EPYC 7763 (loop kernel 1.591), pass on EPYC 9V74 (H15-Q3). aarch64 speed gates: interval pass in all 4 jobs (2 pass, 6 void).
- Local gate 2026-10-21 (Apple M5 Max, not a quiet window): all structural gates pass on aarch64 and statically on x64 SysV and Win64; `sibling-vs-cgoto-a64` pass 0.975; `sibling-vs-match-a64` 1.489, void (call IQR 5.0%).
- Local gate after the T0 follow-up (2026-10-21, Apple M5 Max, load 9-13, so provisional): exit 0. All structural gates pass on aarch64 and statically on x64 SysV and Win64, negative builds included. `sibling-vs-cgoto-a64` pass 0.985; `sibling-vs-match-a64` pass 1.550; `REPORT match-vs-cgoto` 1.522, void.
- T0 (2026-10-21), branch `diag/h15-t0-x64`: runs [39500785277](https://github.com/Brooooooklyn/ep9/actions/runs/39500785277) and [39502380399](https://github.com/Brooooooklyn/ep9/actions/runs/39502380399) (22 x86-64 jobs on 5 CPU models, 1 Neoverse-N2, 2 Apple M2 Pro, cachegrind, disasm-check); run [39503403036](https://github.com/Brooooooklyn/ep9/actions/runs/39503403036) (SysV handler ABI on Windows: old vs new sibling, tests). Local: `disasm-check --all-targets --expect 282` passes on 5 targets (also with nightly `become` on Win64, Linux and aarch64); `ep9_vm` and `ep9_cli` tests pass on aarch64 and on x86_64-apple-darwin (Rosetta).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H15-Q1 | x64 release expansion with the T0 opcode set | answered early (2026-10-21): sibling, on 5 named x64 models (EPYC 7763, 9V74, 9V45; Xeon 8370C, 8573C). Still open: one Intel client model and a gate-host run, by M2 exit (T0 parity) | sibling (this revision) |
| H15-Q2 | Why sibling code pays about 5x more per mispredicted branch on Zen 3 (against the spike's threaded match loop) | only if a threaded match loop comes back; not seen in T0 (EPYC 7763 `branchy`: sibling 1.66-1.72x faster) | `perf stat` on a bare-metal Zen 3 host (hosted AMD VMs have no PMU) |
| H15-Q3 | The cgoto.c compiler is not pinned (clang 18 Linux, 20 Windows, Apple clang 17 macos-15). Win64 on EPYC 7763: loop kernel 1.591x cgoto.c for match (interval fail, run void), mostly because cgoto.c and sibling run that kernel faster there than on Linux. Since the T0 revision the Win64 gate is sibling vs cgoto.c: 1.233 there, max kernel 1.381 | M0 exit item 5 | Pin one LLVM version for cgoto.c on all targets; repeat Win64 on EPYC 7763 with the sibling gate |
| H15-Q4 | macos-15 (Apple M2 Pro, virtual) cgoto.c is 2x slower than sibling. It uses Apple clang 17.0.0; Apple clang 21 and Homebrew clang 23 give the same code and speed locally | M0 exit | covered by the H15-Q3 compiler pin |

## Reopens if

- The aarch64 speed gates fail on the gate host in a quiet window (09 §4).
- ~~The x64 loop check fails on T0: x64 release moves to sibling calls, and this ADR reopens.~~ Fired 2026-10-21; x64 release is sibling calls (this revision).
- On a named x64 model, T0 match is faster than T0 sibling (a PERF-NOISE fail of "match / sibling > 1" on the geomean of the 4 T0 kernels, ≥ 31 rounds).
- A T0 sibling handler loses its tail jump on a tier-1 target (`disasm-check`, RK15) and no DSL fix restores it.
- On a named x64 model, the release expansion fails against cgoto.c (a PERF-NOISE fail with ≥ 31 rounds).
- A handler needs a 5th argument. D8 changes (`become` on stable).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | reopen + draft | First draft. DISPATCH-GATE trigger fired on x64 (CI run 39458801246: 0.812; match beats sibling on a kernel). Release expansion per target (x64: match); gate per target; PERF-NOISE verdicts in the bench; CI speed gates non-blocking with warnings, summary and artifact | H15 agent (M0 workflow) |
| 2026-10-21 | reopen + revision | T0 trigger fired: the T0 match loop is not threaded on x86-64 (2 indirect jumps, need ≥ 79). Sibling is faster on all 22 x64 jobs (5 models) and on arm64. Release expansion: sibling calls on every target. x86-64 handlers use `extern "sysv64-unwind"` (Win64 frames 34 → 12). `--match-loop` is a report for T0. xtask reads Mach-O data in code. H15-Q1 answered (Intel client open), H15-Q2 narrowed, triggers replaced | H15 T0 agent (M0 workflow) |
| 2026-10-21 | rewrite | Follow-up of the T0 revision, no new decision: SP-Dispatch `Variant::RELEASE` = sibling on every target; its x86-64 speed gates are `sibling-vs-cgoto-x64-*`; `match-vs-cgoto` is a report on every target; the spike's `--match-loop` gate stays structural and blocking (Rules). H15-Q3 names the sibling gate. Follow-ups list closed ([decisions-log §10](../decisions-log.md#10-2026-10-21-pr-1-review-fixes-ci-gates-and-h15-on-t0)) | follow-up agent |
| 2026-10-21 | rewrite (ABI bump) | T0 handler count n 282 -> 284 (ABI 3, M2-P0): `CallRuntimeForPair` (0x9d) adds a handler and its cold body adds one more, as `CallRuntime` has (158 opcodes + 123 cold blocks + 3 = `THREADED_HANDLERS`). Rules row T0 and the CI `disasm-check --expect` say 284 (decision M2-DISASM-284) | M2-P0 integration fixer |
| 2026-10-22 | rewrite (as built) | M0 month 2 P2 (M2-P2-ABORT-SYSV64): under `panic = "abort"` the x86-64 handlers and the `Handler` type are plain `extern "sysv64"`; the dev profile keeps `sysv64-unwind` (`cfg(panic = "unwind")`). Without it the abort release build kept only 50 of 282 tail jumps on the 3 x86-64 targets; with it `disasm-check --all-targets --expect 284` equals the base. Code: FIXER-MOVE aea38a3 + d037a95 (P2-INFRA2). The kind is rewrite because the handler ABI is a value code depends on | P2 integration fixer |
