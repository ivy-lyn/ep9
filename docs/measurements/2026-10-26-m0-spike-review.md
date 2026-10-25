# M0 spike review 2 (2026-10-26)

- **Status: the spike review that `cargo xtask m0-exit` reads at the M0 exit run (M3-P5).** m0-exit reads the newest file whose name ends in `-m0-spike-review.md`, and this is that file. GATES-3 passes no `--verdicts` file (plan v8 V8-A4), so the Verdict table in §1 holds a row for each of the 8 spikes, Spike B and SP-Unit too. [Spike review 1](2026-10-21-m0-spike-review.md) (2026-10-21, CI run 39518270586) stays unchanged as the week-4 record. This file was the DOCS draft `2026-10-25-m0-spike-review-draft.md` (decisions M3-PLAN-D35 and M3-PLAN-D38); M3-P5 renamed it and added §1 and §3.
- **Task:** [09-milestones §4](../plan/09-milestones.md#4-spikes) (the gates) and §2.2 (M0 exit items 5 and 11). Rule: a failed spike gate reopens the named ADRs before M0 continues.
- **Evidence base:** the M3-P4 head `a99b014` (its code equals `3749f9b`) and its CI run 40233505423 (§4); the first M3-P4 head `ba95f9b` (the same code; CI run 40228631155); the M3-P3 head `63f115a` (CI run 40177445170, 46 of 46 checks pass; decision M3-P3-CI); and the M3-P4 x86-64 sandbox runs and role reports (§2.1, §2.4, §2.6). Decision IDs are rows of the [decisions log](../decisions-log.md) §12 (month 2) and §13 (month 3), or of the M3-P5 orchestrator notes (M3-P5-V8A19).
- **Not in this review:** the M3-P5 exit run itself, and gate window 4.
- **Result.** No spike gate failed, and this review reopens no ADR. SP-GC, SP-VA, SP-Value, SP-JIT-mem, Spike B and SP-Unit are GO. SP-tsrs-win is recorded. SP-Dispatch is GO with conditions: condition 2 on AMD EPYC 7763 (Zen 3) stays open by the V8-A19 rule, branch 3 (§3). So m0-exit reads item 5 as OPEN on that one cell and item 11 as PASS (§5).

## 1. Verdicts

| Spike | Gate (09 §4) | Evidence (CI run, steps) | Result | Verdict | ADRs |
|---|---|---|---|---|---|
| SP-GC | S1-S7; 0 missed live objects in 10^6 random safepoints per ABI | run 40233505423 (`a99b014`): job `spike gates` × 4, step "SP-GC gate" (`spikes/sp-gc/gate.sh`) | job success on 4 runners; the step exits 1 on any missed live object, so each seed and ABI gave MISSED 0 (§2.1) | GO | H9, H10 (Accepted) |
| SP-VA | cage below 2^47 on 4 targets, plus the code range | run 40233505423: job `build + test` × 4, step "Run cargo test --workspace --locked" (`crates/ep9_platform/tests/sp_va.rs`) | job success on 4 runners (§2.2) | GO | H7 (Accepted) |
| SP-Value | Value, JsResult, word0 = 8 B; barrier = 4 instructions + the cell test | run 40233505423: job `disasm checks` × 4, step "ep9_heap barrier"; the static asserts in `crates/ep9_value/src/lib.rs` (they compile in every build job) | job success on 4 runners (§2.3) | GO | H6, H11 (Accepted) |
| SP-Dispatch | structural gates; speed gates per target (conditions 1-3 of review 1 §3) | run 40233505423: job `spike gates` × 4, step "SP-Dispatch gate", artifacts `sp-dispatch-gate-*` and `zen3-branchy-*`; run 40228631155 (`ba95f9b`, the same code); gate windows 1 and 2 (condition 1) | structure: pass on 4 runners. Condition 1: PASS. Condition 2: open on AMD EPYC 7763 (Zen 3); 3 readings at the M3-P4 code, all void, `branchy` interval above 1.5 in each (§2.4, §3) | GO with conditions (condition 2, §3) | H15 (Accepted; condition 2 is H15-Q5) |
| SP-JIT-mem | generate, W^X, call, unwind, backtrace on 4 targets | run 40233505423: job `spike gates` × 4, step "SP-JIT-mem gate" | job success on 4 runners (§2.5) | GO | H27, H28 (Accepted) |
| Spike B | T0 -> JIT -> T0 round trip ≤ 18 dispatches (D21) | gate window 2 (m2-p4 `c39f782`, gate host): the Verdict row of [spike-b](2026-10-22-spike-b.md); run 40233505423: steps "Spike B structural gate" and "Spike B placement (b) gate" (× 4), "Spike B Win64 unwind" (windows-x64), "spike-native tests (ep9_vm)" (job `build + test` × 4) | RT 14.618 [13.999, 15.006] dispatches (placement (b)); the `spike gates` steps pass on 4 runners; "spike-native tests (ep9_vm)" passes on 4 runners (§2.6) | GO | H14 (Accepted), D21 = placement (b) |
| SP-Unit | the 11 opcodes as a unit blob on (b): load, call, unwind, GC scan, version fallback, on 4 targets | the Verdict row of [sp-unit](2026-10-23-sp-unit.md) (run 40123502730 at `88dacff`); run 40233505423: step "Spike B unit gate" (× 4) and "SP-Unit Win64 unwind" (windows-x64) | job success on 4 runners (§2.7) | GO | H35 (Accepted), H34 (Draft, due M1 exit) |
| SP-tsrs-win | result recorded | run 40233505423: job "SP-tsrs-win (recorded, not blocking)" | job success (§2.8) | recorded | none (D2 note) |

What changed since review 1:

| Spike | Review 1 (2026-10-21) | Evidence since review 1 | What is still open |
|---|---|---|---|
| SP-GC | GO | CI step "SP-GC gate" green on 4 runners at every phase head (§2.1); one false local FAIL fixed in the gate script (M3-P2-SIGPIPE) | none |
| SP-VA | GO | `sp_va` tests in "cargo test --workspace" green on 4 runners at every head; 3 new tests (one process code range, a Windows top-slot fix) (§2.2) | none |
| SP-Value | GO | CI step "ep9_heap barrier" green on 4 runners at every head; no change to `ep9_value` or the barrier check since review 1 (§2.3) | none |
| SP-Dispatch | GO with conditions (§3 of review 1) | Condition 1 PASS twice (gate windows 1 and 2). Condition 2: 13 SPD rows; EPYC 7763 is void at every head that drew it, with an interval fail on `branchy` from m2-p2 on; the only non-void x64 result is a pass on an Intel Xeon (§2.4) | condition 2 on AMD EPYC 7763 (Zen 3), carried to M1 (§3) |
| SP-JIT-mem | GO | CI step "SP-JIT-mem gate" green on 4 runners at every head; the code now lives in the process code range (§2.5) | none |
| Spike B | not run | Gate window 1: placement (a) above 18; gate window 2: placement (b) 14.618 [13.999, 15.006]. x86-64 (b) built in M3-P0 and green in CI on linux-x64 and windows-x64 (§2.6) | none for the gate; the H29 deopt parts stay open to M1 |
| SP-Unit | not run | `gate.sh unit` green on 4 targets (M3-P1-SPUNIT); GO in M3-P2 (M3-P2-SPUNIT-VERDICT); m0-exit item 11 PASS in the GATES-2 dry run (§2.7) | none for the gate; the A2 loader requirement X-SPU-STRICT |
| SP-tsrs-win | recorded (PASS) | the CI job runs at every head; the tsrs pin is the week-1 pin (§2.8) | none |

## 2. Details

### 2.1 SP-GC

- **Gate:** spill trampolines for a64, SysV and Win64; probes S1-S7 as unit tests; 0 missed live objects in 10^6 random safepoints per ABI (`spikes/sp-gc/gate.sh`).
- **CI:** job `spike gates` × 4, step "SP-GC gate". It is green in each phase-head run of §4. The step did not change its checks after review 1.
- **At `a99b014`:** the job `spike gates` passed on 4 runners in run 40233505423. Each 10^6 run of `stress` exits with a failure when it misses a live object, and `gate.sh` then records a FAIL line and exits 1. So a green step means MISSED 0 for each seed and ABI. This review did not copy the per-seed counts from the job logs.
- **Local CI and the x86-64 sandbox:** the `sp-gc` step passes in every local CI pass (for example M2-P4-LOCALCI, M2-P5-LOCALCI) and in the x86-64 sandbox runs.
- **Change after review 1:** M3-P2-SIGPIPE (982e9db). In local CI the win64 disassembly check gave a false FAIL: `grep -q` stopped early, the writer got SIGPIPE (141), and `set -o pipefail` reported it. The check now reads all input. 3 of 3 runs pass, and a wrong pattern still fails. The GC rules did not change.
- **Related evidence (not the gate):** test262 under GC-stress (a collection before every allocation, the heap verifier after each) gives 0 new failures on 4 targets at `63f115a` (the `test262-gc` job, 24 shard jobs; M3-P3-CI). This is M0 exit item 6. After M3-P4 FIXES the x86-64 Linux sandbox run gives 35,505 / 53,572 files under GC-stress (44 skipped), 0 new failures.
- **M3-P4 (GC-stress skip list):** `has-instance-jitted.js` stays on the list (M3-P4-GCSKIP-WIN). FIXES found it CPU-bound: 132,107 full collections, each with a heap verify, 214 G instructions, 10.3 s on mac a64; no cost that only Windows has. A faster GC-stress heap verify is an M1 item. The GC rules did not change.

### 2.2 SP-VA

- **Gate:** the default cage below 2^47 on 4 targets, plus the code range (H7).
- **CI:** `crates/ep9_platform/tests/sp_va.rs` runs in "cargo test --workspace" (job `build + test` × 4). Green at every phase head up to `63f115a`. At `a99b014` (run 40233505423) the job passed on 4 runners (§4).
- **No change since `63f115a`:** `git diff 63f115a a99b014` has no change in `crates/ep9_platform`, `Cargo.lock` or `Cargo.toml`.
- **Changes after review 1 (all in M2-P1):**
  - M2-P1-CR-PLACE (c878197): one process-wide code range, placed by the cage slot search. New tests `sp_va_process_code_range_below_2_47` and `sp_va_code_range_avoids_tsrs_range`.
  - M2-P1-WIN-TOPSLOT (37613b5): on windows-x64 the top slot of the default cage ends above the user space, and VirtualAlloc gave error 87 (about 1 isolate in 4,600 failed in CI run 39632519735). The slot search now skips that slot. New test `sp_va_skips_slot_above_host_limit`.
- **Count:** the file has 12 tests (review 1: 9). `sp_va_skips_refused_slot` runs on Linux only, so macOS and Windows run 11. A local run on aarch64-apple-darwin at `ec4b841` (2026-10-25): 11 passed, 0 failed.

### 2.3 SP-Value

- **Gate:** `ep9_value` static asserts (Value, JsResult, word0 = 8 B); the barrier fast path is 4 instructions plus the cell test.
- **CI:** job `disasm checks` × 4, step "ep9_heap barrier" (`crates/ep9_heap/disasm-barrier.sh`). Green at every phase head, and on 4 runners in run 40233505423. M0 exit item 8 (the static asserts and `cargo check -p ep9_value`) is PASS in every column of the GATES-2 dry run.
- **Changes after review 1:** none. `git log 9b6cd6e..ec4b841` shows no commit on `crates/ep9_value/src/lib.rs` or on the barrier check script, and `git diff 63f115a a99b014` shows no change to them.

### 2.4 SP-Dispatch

Review 1 gave "GO with conditions" (its §3): (1) the a64 speed gates on the gate host in a quiet window, and `sibling-vs-match-a64` must leave "void"; (2) a non-void `sibling-vs-cgoto-x64-sysv` and `-x64-win64` result on each named x64 model; (3) if a speed gate then fails, H15 reopens.

**Structural gates.** The CI step "SP-Dispatch gate" is green on 4 runners at every phase head, and in run 40233505423. Its `gates.txt` on linux-x64 and windows-x64 in that run gives `pass` for `dsl-3-expansions`, `tail-exits-x64-sysv` / `-x64-win64`, `handler-args` and `match-loop-x64-sysv` / `-x64-win64`. The T0 handler check `cargo xtask disasm-check --expect 284` passes on 5 targets (default build; 286 with `spike-native`), for example M3-P0-DISASM. M3-P4 FIXES changed the TestEqual, Wide and ExtraWide handler bodies (PERF3 task 5); the role reports give the same handler stats after each change (284 handlers: 282 tail, 2 exit, 23 merged; the same frame-on-entry lists on 5 targets).

**M3-P4 T0 instruction counts (not an SPD row).** The M3-P4 FIXES T0 fast paths ([04 §2](../plan/04-execution.md#2-t0-techniques-that-reach-parity)) lower the x86-64 cachegrind instructions of the T0 kernels: t0-loop -10.10%, t0-prop -2.97%, t0-call -2.75%; t0-branchy unchanged (x86-64 cachegrind, `bc63674` vs `ec4b841`). These are counts of the T0 interpreter, not the spike's sibling / cgoto.c timing, so they close no condition.

**Condition 1: PASS.** Both a64 speed gates passed on the gate host (Apple M5 Max) with `SP_SPEED=gate`, 15 rounds and cc `zig cc 0.17.0` (pinned, H15-Q3):

| Row | Window | sibling-vs-cgoto-a64 | sibling-vs-match-a64 | tau |
|---|---|---|---|---|
| M2-P4-W1-COND1 | gate window 1 (2026-10-22) | pass | pass | 0.330568 ns |
| M2-W2-COND1 | gate window 2 | pass | pass | 0.336685 ns |

**Condition 2: open on AMD EPYC 7763.** One SPD row per phase head (M2-COND2-CI): CI spikes job, 31 rounds, report mode; artifacts `sp-dispatch-gate-linux-x64` (sysv) and `sp-dispatch-gate-windows-x64` (win64). The limits are geomean ≤ 1.25 and each kernel ≤ 1.5 (sibling / cgoto.c). The result is the interval verdict; geomean and branchy are the medians of the paired ratios.

| Row | Head | CI run | sysv: CPU, result, geomean, branchy | win64: CPU, result, geomean, branchy |
|---|---|---|---|---|
| SPD-P0 | m2-p0 15a68bf | 39604675506 | EPYC 9V45, void, interval pass, 0.988, 0.962 | EPYC 7763, void, interval pass, 1.198, 1.400 (cgoto.c by the system C compiler) |
| SPD-P1 | m2-p1 382429c | 39641297644 | EPYC 9V74, void, pass, 1.173, 1.155 | EPYC 9V45, void, pass, 1.009, 1.011 |
| SPD-P2 | m2-p2 5c9034a | 39677340839 | EPYC 7763, void, **fail**, 1.289, 1.813 | EPYC 7763, void, **fail**, 1.087, 1.710 |
| SPD-P3 | m2-p3 938d2bc | 39706792079 | EPYC 7763, void, **fail**, 1.310, 1.847 | EPYC 9V45, void, pass, 0.998, 1.008 |
| SPD-P4 | m2-p4 c39f782 | 39743241566 | EPYC 7763, void, **fail**, 1.327, 1.876 | EPYC 9V45, void, pass, 1.006, 1.015 |
| SPD-P5 | m2-p5 8970a29 (tag message) | 39798305030 | EPYC 9V74, void, marginal, 1.258, 1.247 | EPYC 7763, void, **fail**, 1.081, 1.559 |
| SPD-M3P0 | 2ba3aa4 | 39846505546 | EPYC 9V45, void, pass, 1.020, 0.999 | EPYC 7763, void, **fail**, 1.117, 1.704 |
| SPD-M3P1 | cbae45d | 39909028691 | EPYC 7763, void, **fail**, 1.308, 1.752 | EPYC 9V74, void, pass, 0.926, 0.995 |
| M3-P2-PROBE1 | 1edad65 (probe branch) | 39929684890 | EPYC 7763, void, **fail**, 1.330, 1.923 | EPYC 7763, void, **fail**, 1.094, 1.634 |
| SPD-M3P2 | 88dacff | 40123502730 | EPYC 7763, void, **fail**, 1.357, 1.855 | EPYC 9V74, void, pass, 0.938, 0.982 |
| SPD-M3P3 | 63f115a | 40177445170 | Intel Xeon Platinum 8573C, **not void** (IQR 0.3-0.7%), pass, 0.926, 0.968 | EPYC 7763, void, **fail**, 1.098, 1.596 |
| SPD-M3P4a | ba95f9b (code = a99b014) | 40228631155 | EPYC 7763, void, **fail**, 1.348, 1.928 | EPYC 7763, void, **fail** (branchy only), 1.085, 1.673 |
| SPD-M3P4 | a99b014 | 40233505423 | EPYC 9V45, void, pass, 1.017, 1.001 | EPYC 7763, void, **fail** (branchy only), 1.088, 1.666 |

The M3-P4 rows in full (from `bench.txt` and `gates.txt` of the artifacts; "void" means a kernel IQR above 5%):

| Row, ABI | CPU | Why void (kernel IQR > 5%) | geomean median [interval] | branchy median [interval] | Interval verdict |
|---|---|---|---|---|---|
| SPD-M3P4a sysv | AMD EPYC 7763 | loop 18.0%, branchy 14.1%, call 10.4% | 1.348 [1.306, 1.395] | 1.928 [1.835, 2.002] | geomean and branchy above their limits |
| SPD-M3P4a win64 | AMD EPYC 7763 | loop 5.8%, branchy 8.0% | 1.085 [1.068, 1.101] | 1.673 [1.621, 1.715] | branchy above 1.5; geomean, loop and call pass |
| SPD-M3P4 sysv | AMD EPYC 9V45 | loop 8.6% | 1.017 [1.005, 1.031] | 1.001 [0.996, 1.004] | pass |
| SPD-M3P4 win64 | AMD EPYC 7763 | loop 7.5%, branchy 14.4%, call 7.7% | 1.088 [1.052, 1.115] | 1.666 [1.525, 1.692] | branchy above 1.5; geomean, loop and call pass |

What the rows show:

- SPD-M3P3 sysv is the only non-void x64 result. It passes, on an Intel Xeon Platinum 8573C. Condition 2 asks for a non-void result per named model, so this closes no AMD model.
- No win64 result is non-void.
- AMD EPYC 7763 (Zen 3) is void at every head that drew it. From m2-p2 on (cgoto.c by zig cc 0.17.0) each of these results has an interval fail on `branchy`, on sysv and on win64. At the M3-P4 code it drew EPYC 7763 3 times (SPD-M3P4a sysv and win64, SPD-M3P4 win64), with branchy 1.928, 1.673 and 1.666. EPYC 9V45 and 9V74 give interval pass (SPD-P5 sysv on 9V74: marginal).
- SPD-P2-AB: on EPYC 7763 the sibling time did not move between m2-p0 and m2-p2; the cgoto.c time fell by 22% with zig cc 0.17.0.

**Condition 3 and H15.**

- M2-P3-SPD-D12 (2026-10-22): the fail path fired for x64 sysv on EPYC 7763 (2 heads in a row). The gate kernel and the limits stay. H15 is reopened for Zen 3, and H15-Q2 is due before the M0 exit. M2-P5-PLAN-D19 makes H15-Q2 cover sysv and win64.
- The H15-Q2 study (M3-P2 PERF3; M3-P2-H15-Q2, M3-P2-PROBE1): hypothesis H1 (the gap is the cgoto.c `JumpIfFalse` form) is false. All measured cgoto.c forms give about 1.6x to 1.9x. On the same CPU, T0 sibling is 1.76x (sysv) and 1.95x (win64) faster than T0 match. T0 against a matched C baseline on Zen 3 was not measured (correction R3-4).
- H15 was accepted on 2026-10-25 on the condition-1 output, with condition 2 as the Open question H15-Q5 (owner: the orchestrator's CI-recorded run; M2-H15-ACCEPT, M3-P3-ACCEPT-7).
- M3-P3-V8A19 (GATES-2): branch 3. M3-P5-V8A19 (before the exit run): branch 3 again. §3 gives the rule result.

**Report only (not a gate):** the "SP-Dispatch Zen 3 branchy report" step (`spikes/sp-dispatch/zen3-branchy.sh`) runs on linux-x64 and windows-x64 since M3-P2 (WS2-D5). It times the spike's sibling against 3 forms of cgoto.c (as-is, branch, select), 31 rounds, and T0 sibling against T0 match (median of 11). At SPD-M3P3 it gives, on EPYC 7763 (win64), branchy sibling / cgoto.c 1.747, 2.306 and 1.617 for the forms as-is, branch and select (all void). At the M3-P4 code (the `zen3-branchy-*` artifacts):

| Run, ABI | CPU | branchy sibling / cgoto.c: as-is, branch, select (median [interval]) | geomean: as-is, branch, select | Verdict of each form | T0 match / sibling: t0-branchy, t0-loop |
|---|---|---|---|---|---|
| 40228631155 sysv | AMD EPYC 7763 | 1.912 [1.767, 2.039], 1.804 [1.710, 1.975], 1.868 [1.815, 1.962] | 1.365, 1.305, 1.335 | void, interval fail | 1.297 (1280.3 vs 1660.9 ms), 1.346 |
| 40228631155 win64 | AMD EPYC 7763 | 1.660 [1.615, 1.692], 2.049 [2.009, 2.119], 1.721 [1.664, 1.754] | 1.063, 1.146, 1.087 | void, interval fail | 2.113 (1054.5 vs 2228.4 ms), 1.920 |
| 40233505423 sysv | AMD EPYC 9V45 | 1.003 [0.996, 1.006], 1.002 [0.994, 1.010], 0.992 [0.985, 1.001] | 1.001, 1.007, 1.001 | void, interval pass | 1.329 (621.1 vs 825.6 ms), 1.394 |
| 40233505423 win64 | AMD EPYC 7763 | 1.723 [1.700, 1.798], 2.114 [2.089, 2.185], 1.734 [1.679, 1.774] | 1.116, 1.201, 1.117 | void, interval fail | 2.122 (1072.3 vs 2275.0 ms), 1.960 |

On EPYC 7763 every cgoto.c form leaves branchy at 1.66 to 2.11, and T0 sibling stays faster than T0 match (1.30x to 2.12x). On EPYC 9V45 (not Zen 3) all forms are near 1.0. The `commit:` line of these logs names `9625bcd` (run 40228631155) and `76785eb` (run 40233505423), not the head commits; these commits are not in the local clone, and this review did not check them.

### 2.5 SP-JIT-mem

- **Gate:** one generated function per target; W^X toggle; call; unwind and backtrace (DWARF CFI; `RtlAddGrowableFunctionTable`) on 4 targets.
- **CI:** job `spike gates` × 4, step "SP-JIT-mem gate". Green at every phase head, and on 4 runners in run 40233505423.
- **Changes after review 1 (M2-P1):** f6e95a4 puts the generated code in a chunk of the process code range (H07-Q6), and 2bb6b73 moves the last code users to `CodeRange::process()`.
- **Related:** the Win64 unwind of JIT-like code is also checked by the steps "Spike B Win64 unwind" (since m2-p4) and "SP-Unit Win64 unwind" (since M3-P2), and by the live Win64 unwind test of x86-64 placement (b) (M3-P0-X64-CI).

### 2.6 Spike B

- **Gate (D21):** keep native-stack JIT frames if one T0 -> JIT -> T0 round trip costs ≤ 18 release-expansion dispatches of the loop kernel on the same host ([protocol](2026-10-22-spike-b-protocol.md)).
- **Gate window 1** (2026-10-22, head 938d2bc; M2-P4-W1-D21): placement (a) with the decode-free entry (M2-P3-D21-ENTRY): RT 19.069 [18.645, 19.672] dispatches, above 18. M2-D21-FAIL applies: P4 builds placement (b).
- **Gate window 2** (m2-p4 c39f782; M2-W2-D21-B): placement (b), gate config B: RT 14.618 [13.999, 15.006] dispatches (k = 5, 15 rounds, IQR t_B 2.7%, t_C 2.3%, load guard PASS, gate host). Reports in the same rounds: A0 (T0 -> T0) 14.473, RT_A 18.604, RT_AH 21.110. Cachegrind, Ir per round trip above C: A0 143.6, B 147.8, AH 180.1, A 240.7.
- **The Verdict row** of [2026-10-22-spike-b.md](2026-10-22-spike-b.md) says GO (gate window 2, placement (b)). The GATES-2 dry run read it through `--verdicts`; GATES-3 reads the Spike B row of §1 of this file.
- **What changed as a result:** D21 moved to placement (b). H14 was reopened, H14-Q5 filled, and the H29 "Reopens if" row fired (P5 docs sync). H14 was accepted on 2026-10-25 (M3-P2-ADR). SP-Unit moved to M3-P1 and builds on (b) (M2-P4-SPUNIT-M3).
- **x86-64 placement (b)** (M3-P0; M3-P0-X64B): built in `spikes/spike-b/src/jit_b/x64.rs` with placement (a)'s frame and the T0 handler ABI. The D25 fallback is not used. Evidence: the x86-64 sandbox runs of local CI (M3-P0-X64-A3: `structural-b: PASS on x86_64`, 0 SKIP lines), CI run 39846505546 on linux-x64 and windows-x64 with the live Win64 unwind test 3 of 3 (M3-P0-X64-CI). An x86-64 bench report (sandbox, not the gate host, void; M3-P0-X64-A11): RT 13.316 [12.790, 14.008]; in cachegrind a (b) round trip costs the same Ir as A0. `gate.sh b` fails on a SKIP line since M3-P0 (P0-D9).
- **Fixes after the gate:** the D14 gap (the native entries used the plain `bind_this`) is closed (M3-P0-X64B). CX-JIT1: the (b) entry dropped the incoming accumulator on both arches; fixed with a new test (M3-P0-FIX-ACC). M3-P4 fix round 1 (M3-P2-OPEN item 6 (M3-P2-STOP)): `native::run_b` (a call from Rust of a (b) function, `spike-native` build) now takes its budget unit before it checks the frame push, so a call whose frame does not fit takes 1 unit, not 0 (H23 Facts "Interrupt poll"); the new `spike-native` test `a_b_call_from_rust_that_does_not_fit_takes_one_unit` fails without the fix. The round trip code and the gate do not change.
- **CI:** steps "Spike B structural gate" (since m2-p2), "Spike B placement (b) gate" (since m2-p4), "Spike B Win64 unwind" (windows-x64, since m2-p4) and "spike-native tests (ep9_vm)" (since m2-p2). All green at every head after they were added, up to `63f115a`. In run 40233505423 the `spike gates` job passed on 4 runners, and "spike-native tests (ep9_vm)" (job `build + test`) passed on 4 runners. `crates/ep9_vm/src/native.rs` changed after `63f115a` (the `run_b` budget unit above), and the windows-x64 job of run 40233505423 is the first Windows pass of that step with the change.

### 2.7 SP-Unit

- **Gate (09 §4):** the 11 Spike B opcodes as a unit blob on placement (b): load, call, unwind, GC scan and fall back on a version mismatch, on 4 targets. Time box: 2 weeks beyond its budget. It moved from month 2 to M3-P1 by M2-D21-FAIL.
- **Built in M3-P1** (SP-UNIT, 9acc400; [2026-10-23-sp-unit.md](2026-10-23-sp-unit.md)): `spike-b unit`, checks (a)-(g). Unit text is placement (b) code plus the unit-data register (H35-Q9). Text with 0 relocations; one text for 3 isolates; RW copy per isolate with strong and weak root ranges; two threads; unwind self-test, backtrace, exception and termination through unit frames; 21 mismatch cases that end in their S3 outcome.
- **Runs (M3-P1-SPUNIT):** mac a64 and linux a64 (the role and local CI), linux x64 (x86-64 sandbox request and local CI), windows-x64 (CI probe run 39886596197).
- **Verdict row** (M3-P2-SPUNIT-VERDICT, 913a532): GO in [2026-10-23-sp-unit.md](2026-10-23-sp-unit.md). Evidence: CI run 40123502730 at 88dacff, step "Spike B unit gate" with its "unit: PASS on" line on all 4 runners, and the windows-x64 step "SP-Unit Win64 unwind". In run 40233505423 at `a99b014` the `spike gates` job, with these steps, passed on 4 runners.
- **Win64 static check** (WIN-STATIC-2; WS2-D1, WS2-D3): a model of RtlVirtualUnwind checks 173 states over 10 unit functions; 2 broken UNWIND_INFOs must fail. Its first native Windows run passed (probe 39929684890).
- **Review fixes:** SPUNIT-1 (the heap kept root ranges on freed words after a LOAD ERROR) is fixed with a two-layer link check (SPUNIT-FIX-2LAYER). X-SPU-STRICT (the spike SOURCE fallback compiles sloppy and does not read the header STRICT flag) is (c): a requirement on the A2 loader (A5S2-XSPU-STRICT).
- **Exit item 11:** PASS in all 5 columns of the GATES-2 dry run at `63f115a`.

### 2.8 SP-tsrs-win

- **Gate:** the result is recorded (D2 note). Review 1: PASS, 0 local patches, 12 of 12 runs ([data](2026-10-20-sp-tsrs-win.md)).
- **CI:** the job "SP-tsrs-win (recorded, not blocking)" runs in every CI run. The job has `continue-on-error: true`, so its own conclusion must be read from the run, not from the workflow result. In run 40233505423 at `a99b014` its conclusion is success (§4).
- **Pin:** `third_party/tsrs.commit` has not changed since week 1 (ea31dcb), so the smoke tests run the same tsrs code as in review 1.

## 3. Open condition: SP-Dispatch condition 2 on AMD EPYC 7763 (Zen 3)

Before the exit run the orchestrator applied the V8-A19 rule again (decision M3-P5-V8A19; the first time was M3-P3-V8A19 at GATES-2). This is the result, as a statement. The owner may override it.

- **Branch 1 (a non-void PASS of condition 2 on EPYC 7763): not met.** At the M3-P4 code CI drew EPYC 7763 3 times: SPD-M3P4a sysv, SPD-M3P4a win64 and SPD-M3P4 win64. All 3 are void (a kernel IQR above 5%), and in each the `branchy` interval is above the 1.5 limit: 1.928 [1.835, 2.002], 1.673 [1.621, 1.715] and 1.666 [1.525, 1.692] (§2.4). No SPD row has a non-void result on EPYC 7763.
- **Branch 2 (the data puts the gap in the cgoto.c baseline, not in T0): not met.** No new data moves the gap to cgoto.c. On EPYC 7763 the 3 cgoto.c forms of the Zen 3 branchy report give branchy sibling / cgoto.c of 1.66 to 2.11 at the M3-P4 code (§2.4). This is the same result as M3-P2-H15-Q2, which stands.
- **Branch 3 applies.** The gate stays: the kernel and the limits (geomean ≤ 1.25, each kernel ≤ 1.5) do not change. Exit item 5 stays OPEN on one named cell: SP-Dispatch condition 2 on AMD EPYC 7763 (Zen 3), sysv and win64. M0 exits with this named exception, carried to M1, and the milestone report says so. H15-Q5 stays open (owner: the orchestrator's CI-recorded run).
- **Not part of the cell:** other x64 models do not close it, and they do not count against it. EPYC 9V45 gives interval pass at `a99b014` (sysv, void), and Intel Xeon Platinum 8573C gives a non-void pass at `63f115a` (sysv).

For this reason the SP-Dispatch verdict in §1 keeps the word "condition". m0-exit reads it as OPEN, not PASS and not FAIL (§5).

## 4. CI runs at the phase heads since review 1

Each run has all 4 tier-1 runners. A green run includes the spike gates job (SP-GC, SP-Dispatch, SP-JIT-mem and the Spike B steps), the disasm job (the barrier) and the build + test job (SP-VA, spike-native tests).

| Head | Commit | CI run | Checks |
|---|---|---|---|
| m2-p0 | 15a68bf | 39604675506 | 22 of 22 |
| m2-p1 | 382429c | 39641297644 | 22 of 22 |
| m2-p2 | 5c9034a | 39677340839 | 22 of 22 |
| m2-p3 | 938d2bc | 39706792079 | 22 of 22 |
| m2-p4 | c39f782 | 39743241566 | 22 of 22 |
| m2-p5 | 8970a29 | 39798305030 | 22 of 22 |
| M3-P0 | 2ba3aa4 | 39846505546 | 22 of 22 |
| M3-P1 | cbae45d | 39909028691 | 22 of 22 |
| M3-P2 | 88dacff | 40123502730 | 22 of 22 |
| M3-P3 | 63f115a | 40177445170 | 46 of 46 (22 + the 24 `test262-gc` shard jobs) |
| M3-P4 (first push) | ba95f9b | 40228631155 | 46 jobs. 43 success; build + test (linux-x64) and (windows-x64) cancelled at the 60-minute limit in the step "feature matrix", with no failed test (M3-P4-CI-TIMEOUT). The orchestrator recorded the run while build + test (macos-arm64) still ran |
| M3-P4 | a99b014 | 40233505423 | 46 of 46 (build + test limit 90 minutes; build + test (windows-x64) took 72 minutes) |

Run 40233505423: the job list of the run (scratchpad m0m3/p5/ci-40233505423-jobs.txt, M3-P5 orchestrator notes) shows 46 of 46 jobs success. Each job that this review cites is in it: spike gates × 4, disasm checks × 4, build + test × 4, SP-tsrs-win, and the test262 and test262 GC-stress jobs. This review did not read the step logs. In the `build + test` job, the steps "Run cargo test --workspace --locked" (SP-VA) and "spike-native tests (ep9_vm)" (Spike B) have no `continue-on-error` (`.github/workflows/ci.yml`), so a job success means that both steps passed. m0-exit reads both steps from the run for the Windows column of item 5.

## 5. How m0-exit reads this file

- m0-exit reads each table with a cell "Spike" and a cell "Verdict" in its header row (`verdict_table` in `tools/xtask/src/m0_exit.rs`). In §1 each Spike cell names one spike. The second table of §1 and the tables of §2-§4 have no header cell that is exactly "Verdict" (the §2.4 cells "Interval verdict" and "Verdict of each form" do not match), so m0-exit does not read them.
- The word rule (`spike_status`): a verdict that contains "fail" or "no-go" (any letter case) is FAIL. A verdict is PASS only if it starts with "go" and has no "condition" in it; for SP-tsrs-win a verdict that starts with "recorded" is PASS too. Anything else is OPEN.
- So items 5 and 11 read: SP-GC, SP-VA, SP-Value, SP-JIT-mem, Spike B: PASS; SP-tsrs-win: PASS (recorded); SP-Dispatch: OPEN (§3). Item 5 is OPEN on that one cell; item 11 (SP-Unit) is PASS. The Windows column of item 5 also needs the 7 CI steps of the run (§4).
- GATES-3 passes no `--verdicts` file (V8-A4). A `--verdicts` file would override the rows of this file.

## 6. Change log

| Date | Change |
|---|---|
| 2026-10-25 | Draft body (M3-P4 DOCS, step 1): the evidence at `ec4b841`. No Verdict cells |
| 2026-10-26 | M3-P4 DOCS step 2: the M3-P4 work that touches spikes (the GC-stress run and M3-P4-GCSKIP-WIN, the T0 handler stats, the `run_b` budget unit) and a placeholder for the M3-P4 head CI run. No Verdict cells |
| 2026-10-26 | M3-P5: renamed from `2026-10-25-m0-spike-review-draft.md` to this name; spike review 2. Added the Verdict table (§1), the open condition by M3-P5-V8A19 (§3), the SPD rows SPD-M3P4a and SPD-M3P4 and the Zen 3 report at the M3-P4 code (§2.4), and the M3-P4 CI runs (§4) |
| 2026-10-26 | M3-P5 fix: run 40233505423 is 46 of 46 success (its job list), so build + test (windows-x64) passed. The SP-VA and Spike B rows (§1, §2.2, §2.6) and §4 now say "4 runners", and the placeholder CI-A99-PENDING is gone. §5 says which header cell m0-exit matches |
