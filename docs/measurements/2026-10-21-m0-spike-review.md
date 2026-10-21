# M0 spike go/no-go review (2026-10-21): no red gate

- Task: [09-milestones §8](../plan/09-milestones.md#8-m0-first-month-week-by-week) week 4 "Spike go/no-go review". Rule ([09 §4](../plan/09-milestones.md#4-spikes)): a failed spike gate reopens the named ADRs before M0 continues.
- Evidence: CI run [39518270586](https://github.com/Brooooooklyn/ep9/actions/runs/39518270586), the last run on the week-4 base tree. Its head `9b6cd6e` (PR #2) has tree `18fcb648`, the same tree as `main` at `dabc38a`. All 21 jobs ended with success.
- The push run on `main` at `dabc38a` ([39519345205](https://github.com/Brooooooklyn/ep9/actions/runs/39519345205)) started no job: "recent account payments have failed or your spending limit needs to be increased". So it holds no result. The week-4 phase merges need CI; this is an owner action.
- **Result.** Every gate that ran is green. No ADR reopens. SP-Dispatch is GO with conditions (§3).

## 1. Verdicts

| Spike | Gate (09 §4) | Evidence | Result | Verdict | ADRs |
|---|---|---|---|---|---|
| SP-GC | S1-S7; 0 missed live objects in 10^6 random safepoints per ABI | job `spike gates` × 4, step "SP-GC gate" (`spikes/sp-gc/gate.sh`) | PASS on 4 runners (§2.1) | GO | H9, H10 stay |
| SP-VA | cage below 2^47 on 4 targets, plus the code range | job `build + test` × 4: `crates/ep9_platform/tests/sp_va.rs` | PASS on 4 runners (§2.2) | GO | H7 stays (its rewrite to one cage per isolate is not a reopen) |
| SP-Value | Value, JsResult, word0 = 8 B; barrier = 4 instructions + the cell test | `crates/ep9_value/src/lib.rs:44-55` (compiles on 4 targets); job `disasm checks` × 4, step "ep9_heap barrier" | PASS on 4 runners (§2.3) | GO | H6, H11 stay |
| SP-Dispatch | structural gates; DISPATCH-GATE (T0) speed gates per target | job `spike gates` × 4, step "SP-Dispatch gate", artifacts `sp-dispatch-gate-*`; [README](../../spikes/sp-dispatch/README.md) | structure: PASS on 4 runners; speed: no fail, no counted pass (§2.4) | GO with conditions (§3) | H15 stays |
| SP-JIT-mem | generate, W^X, call, unwind, backtrace on 4 targets | job `spike gates` × 4, step "SP-JIT-mem gate"; [measurements](2026-10-21-sp-jit-mem.md) | PASS on 4 runners (§2.5) | GO | H28, H27 stay |
| SP-tsrs-win | result recorded | job `SP-tsrs-win`, artifact `sp-tsrs-win-log` | PASS, 0 local patches, 12 of 12 runs ([data](2026-10-20-sp-tsrs-win.md)) | recorded | none (D2 note) |
| Spike B, SP-Unit | — | M0 month 2 | not run | not reviewed | — |

## 2. Details (CI run 39518270586)

### 2.1 SP-GC

| Runner | Probes S1-S7 (`probes::*`) | 10^6-safepoint runs, MISSED | Negative controls |
|---|---|---|---|
| macos-arm64 | 12 probe tests ok | aapcs64 × 3 seeds: 0, 0, 0 | no-spill, exact-only, no-end: misses as expected |
| linux-arm64 | 12 probe tests ok | aapcs64 × 3: 0, 0, 0 | as expected |
| linux-x64 | 12 probe tests ok | sysv64 × 3: 0, 0, 0; win64 × 3: 0, 0, 0 | as expected |
| windows-x64 | 12 probe tests ok | win64 × 3: 0, 0, 0 | as expected |

Each 10^6 run: 1,000,001-1,000,018 safepoints, about 9.0-9.1 million live checks, frame depth up to 16.

### 2.2 SP-VA

All `sp_va_*` tests pass: 8 on macOS arm64 and Windows x64, 9 on Linux (the extra one, `sp_va_skips_refused_slot`, needs seccomp). They cover the default cage below 2^47, the code range at the cage end, the tsrs range, used and refused slots, drop, a 39-bit limit, bad input and a fault on an uncommitted page.

### 2.3 SP-Value

| Runner | `disasm-barrier.sh` |
|---|---|
| macos-arm64, linux-arm64 | PASS: cell test 3 + header check 4 (`ldrb ldrb cmp b.ls`) + store (`str`) |
| linux-x64, windows-x64 | PASS: cell test 3 + header check 4 (`movzbl movzbl cmpb jbe`) + store (`movq`) |

### 2.4 SP-Dispatch

Structural gates, all PASS: `dsl-3-expansions`, `tail-exits-a64` / `-x64-sysv` / `-x64-win64`, `handler-args`, `match-loop-x64-sysv` / `-x64-win64`. In the `disasm checks` jobs, `cargo xtask disasm-check -p ep9_vm --expect 282` passed on the 4 hosts (282 handlers, at most 4 arguments).

Speed (wall time, 31 paired rounds, `SP_SPEED=report`; hosted runners never block, decision SPEED-CI):

| Runner, CPU | Gate | Median (interval) | Verdict |
|---|---|---|---|
| macos-arm64, Apple M2 Pro (Virtual), load 14.0 → 5.8 | sibling-vs-cgoto-a64 | 0.481 (0.478-0.491) | void: branchy IQR 5.5% (interval: pass) |
| macos-arm64 | sibling-vs-match-a64 | 2.116 (2.085-2.138) | void: call IQR 5.9% (interval: pass) |
| linux-arm64, Neoverse-N2 | sibling-vs-cgoto-a64 | 1.001 (0.981-1.023) | void: loop IQR 9.1% (interval: pass) |
| linux-arm64 | sibling-vs-match-a64 | 1.734 (1.706-1.747) | pass |
| linux-x64, AMD EPYC 7763 | sibling-vs-cgoto-x64-sysv | 1.165 (1.134-1.195) | void: kernel IQRs 6.5-16.9% (interval: pass) |
| windows-x64, AMD EPYC 7763 | sibling-vs-cgoto-x64-win64 | 1.151 (1.131-1.187) | void: kernel IQRs 5.6-11.1% (interval: pass) |

Gate host (Apple M5 Max), from the README: sibling-vs-cgoto-a64 pass 0.975 (2026-10-21, provisional host); sibling-vs-match-a64 1.489, void (call IQR 5.0%). The reports `match-vs-cgoto` (every target) and `sibling-vs-match` on x86-64 are reports, not gates.

### 2.5 SP-JIT-mem

| Runner | Gate ids | W^X pair cost (default mode) |
|---|---|---|
| macos-arm64 | all PASS; unwind-backtrace 8 checks; control (no unwind info) PASS | map-jit 11.4 ns |
| linux-arm64 | all PASS; 8 checks; control PASS | protect 3,655 ns (dual-map 11.2 ns, not default) |
| linux-x64 | all PASS; 8 checks; control PASS | protect 7,160 ns (dual-map 14.1 ns, not default) |
| windows-x64 | all PASS; 4 checks; control SKIP (an unregistered x64 frame looks like a leaf) | protect 1,365 ns |

## 3. Conditions for SP-Dispatch (before M0 exit)

1. Run the a64 speed gates on the gate host in a quiet window (10-quality §3: load guard passes, ≥ 15 paired rounds). sibling-vs-match-a64 must leave "void". This review ran at load 15 on the shared M5 Max (other agents were building), so it did not re-run them.
2. Get a non-void result for `sibling-vs-cgoto-x64-sysv` and `-x64-win64` on each named x64 model, on a quiet host. A hosted result holds only for its CPU model.
3. If a speed gate then fails, H15 reopens (09 §4) before M0 continues.

## 4. Proposed decision rows (for decisions log §11)

| ID | Decision | Reason | Evidence |
|---|---|---|---|
| W4-SPIKE-REVIEW | M0 spike review: SP-GC, SP-VA, SP-Value, SP-JIT-mem GO; SP-Dispatch GO with conditions (W4-SPIKE-DISPATCH); SP-tsrs-win recorded as PASS; Spike B and SP-Unit not reviewed. No ADR reopens | Every gate step that ran on the 4 tier-1 runners passed; no speed gate failed | CI run 39518270586 (tree = `dabc38a`); this file |
| W4-SP-TSRS-WIN | SP-tsrs-win: PASS (0 local patches, 12 of 12 CI runs). By D2, `ep9 check` may support Windows; `ep9c build --hints` waits for the A1h re-check | The gate's `ep9 check` part passed | [data](2026-10-20-sp-tsrs-win.md) |

The orchestrator re-checks this review on the final week-4 CI run (week-4 plan §12): the same steps on 4 runners. A FAIL line reopens its ADR.
