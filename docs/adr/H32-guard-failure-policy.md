# H32: Guard-failure policy per mode

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the per-mode table of 04 §7 and the jitless budgets of 05 §10. Nothing is built: no tier emits a guard yet |
| Fix by | Frozen M1 exit; applied A3, A4 |
| Freeze point | M1 exit, the policy (README §2). Applied in A3 and A4, after M1 exit, and the budgets are tuned in A4 (05 §10), so README §3.4 applies: H32-Q5 |
| Origin | R16-D11; plan status N |
| Crates | later `ep9_opt` (guard lowering per mode), `ep9_baseline` (A3 IC slow paths), `ep9_aot` (SLOWPATH variants); `ep9_vm` (deoptimizer, exit counters, PIN through the dispatch slot) |
| Owner (plan role) | AOT senior (A3, A4; SP-Facts reopens H32, 09 §4); CORE T2 owner (JIT-mode guards) |
| Validated by | SP-Facts (M1) for the IR policy; hint chaos (A4), mode x tier differential (A3), the RK33 exit-counter check on JITLESS-SET |
| Plan sources | [02 §6 H32](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §7](../plan/04-execution.md#7-guard-failure-policy-per-mode-h32), [05 §10](../plan/05-aot.md#10-jitless-guard-policy-no-deopt-loops), [05 §4](../plan/05-aot.md#4-type-trust-levels), [H29](H29-deopt-contract.md), [H31](H31-fuse-registry.md) |

## Context

- With no JIT, nothing re-profiles or recompiles, so a deopt can repeat forever. ART emits no deopt in AOT code for this reason ("forever deoptimizations") [R11][R14].
- TS hints are unsound by design, so a hint failure must never change program output (N11, [R12]).
- Depends on: H29 (DEOPT, exit counters), H30 (facts and their source), H31 (fuses), H14 (dispatch slots: PIN writes one slot), H33 (hybrid re-tier).
- Depended on by: H35 (SLOWPATH variants and RW counters in units), A3 and A4 (applied), H24 (breakpoints deopt to T0).

## Decision

Copied from the plan row (02 §6): JIT: DEOPT + exit counter. Hybrid: DEOPT → instrumented re-tier. Jitless: SLOWPATH or DEOPT within budget, then PIN to baseline-AOT. Never throw on hint failure. THROW only in SOUND modules at boundaries (A5, D17 c). Rows reserved for D17 (d): VERIFIED region entry-check failure → SLOWPATH from the start; interior-check failure → DEOPT with frame state; fuse pop → lazy deopt + slot swap. Never THROW.

| Fact (04 §7) | JIT mode | hybrid mode | AOT-jitless mode |
|---|---|---|---|
| PROVEN | No guard | No guard | No guard |
| RUNTIME | Guard -> DEOPT + exit counter | Same | n/a (no runtime compiler) |
| PROFILED | Guard -> DEOPT | Guard -> DEOPT -> instrumented re-tier (H33) | Guard -> SLOWPATH, or DEOPT within budget, then PIN to baseline-AOT |
| HINTED | As PROFILED, at lower confidence | As PROFILED | As PROFILED |
| Global invariant | InvalidatingFuse: lazy deopt + dispatch-slot swap | Same | GuardFuse (load + compare) or InvalidatingFuse (lazy deopt patches no code) |
| SOUND module boundary (A5 only, D17 c) | THROW | THROW | THROW |
| VERIFIED region (A5v only, D17 d; reserved) | entry fail: SLOWPATH from the start; interior fail: DEOPT with frame state; fuse pop: lazy deopt + slot swap | Same | Same |

**Jitless budgets (05 §10; est., tuned in A4).**

```text
 AOT-jitless: a PROFILED or HINTED guard in optimized-AOT code fails
   site exit counter (RW) += 1 --- < 4, or the site has no SLOWPATH variant ---> DEOPT: rebuild a T0 frame at the bytecode offset; function exits += 1
        | >= 4 and the site has a SLOWPATH variant                               | function exits >= 16
        v                                                                        v
   SLOWPATH: generic IC call inside the AOT code                                 PIN: dispatch slot -> aot-baseline
   (the site ignores its fact from now on)                                       (-> interp if the function has no baseline-AOT)
```

- A site with 4 guard failures (est.) stops using its hint, in every mode (05 §4).
- A site without a SLOWPATH variant keeps deopting until the function is pinned (05 §10).

**As built (e719ffa).** The PIN targets exist as dispatch-slot kinds: `Interp = 0`, `AotBaseline = 3` (`crates/ep9_vm/src/dispatch.rs:21-32`); "tier-up, unit load, deopt-all, PIN ==> write one slot" (`crates/ep9_vm/src/dispatch.rs:5`; H14 dispatch slots, `docs/adr/H14-frames-and-dispatch.md:69`). No tier emits a guard, and there is no exit counter or deoptimizer (H29).

## Rules

| Rule | Check | From |
|---|---|---|
| A hint failure never throws and never changes program output | hint chaos fuzzing: random wrong hints and stale profiles, output never changes (10-quality §4); hint soundness corpus (10-quality §2) | A1h; A4 |
| THROW appears only at SOUND module boundaries, outside test262 mode, and only if the D17 rule picks (c) | M1: Spike C / SP-Facts (the IR has no THROW policy for predicted facts); test262 runs in every mode (G12) | M1; A5 |
| In JIT mode a failed RUNTIME, PROFILED or HINTED guard deopts to T0 and bumps an exit counter | M1: Spike C / SP-Facts (one eager deopt, tier-differential test) | M1 |
| In AOT-jitless mode a site with a SLOWPATH variant deopts at most its site budget (4, est.), then takes SLOWPATH; a site without one keeps deopting until its function reaches the function budget (16 exits, est.) and is PINned; no function deopts past its budget | RK33 check: exit counters reach their budget on JITLESS-SET (10-quality §7); mode x tier differential (10-quality §4) | A3; A4 |
| PIN writes only the dispatch slot (to aot-baseline, or interp with no baseline-AOT); it patches no code | `dispatch::tests::*` and `tests::interp::dispatch_slots_and_code_registry` ("a tier change writes one slot", H14 Rules); unit text is RX-only (H35) | M0 wk3 (slot write); A3 (PIN) |
| Global invariants use GuardFuse or InvalidatingFuse per the table; an InvalidatingFuse pop in jitless mode swaps the slot and patches no code | M1: Spike C / SP-Facts; H31 fuse tests | M1; M1-M2 |
| The policy is picked per mode in the IR (H30), not in the backend | M1: Spike C / SP-Facts | M1 |

## Consequences

- **Gains.** No deopt loop in jitless mode. Wrong hints cost speed, never correctness. One table drives guard lowering in every mode.
- **Costs.** SLOWPATH variants grow AOT code size (SIZE-REPORT). PIN gives up optimized code for the rest of the run.
- **Follow-ups.** Budgets tuned in A4. The VERIFIED rows are built only if the D17 rule picks (d) at M7 exit [R18].

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | ART emits no deopt in AOT code; it uses polymorphic inlining with a generic fallback, because AOT deopts can loop forever | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R11] | ART's offline inline caches: polymorphic inlining with a fallback and no deopt ("forever deoptimizations") | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R16] | D11: SLOWPATH or deopt within an exit budget, then pin to baseline-AOT (ART inliner.cc:614-636) | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R12] | TS types lie at run time in 24 crafted hole classes | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R18] | VERIFIED rows: entry fail SLOWPATH, interior fail DEOPT with frame state, fuse pop lazy deopt; the SOUND THROW row becomes unused | [../research/18-sound-regions.md](../research/18-sound-regions.md) |

## Validation

- M1: SP-Facts (policy in the IR); a failed gate reopens H30-H34 (09 §4).
- A3: mode x tier differential. A4: hint chaos, the jitless budgets on JITLESS-SET, SIZE-REPORT for SLOWPATH variants.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H32-Q1 | "HINTED: as PROFILED, at lower confidence": how confidence changes the decision (speculate or not, budget) | M1 (with H30-Q1) | Lower confidence = a higher speculation threshold; the same budgets |
| H32-Q2 | Which guard kinds get a SLOWPATH variant in optimized-AOT code, given the size cost | A4 | Shape, call-target and tag guards on hot sites only; SIZE-REPORT decides |
| H32-Q3 | Does a RUNTIME-fact guard in JIT mode also use a per-site budget, or only the T2 recompile threshold? | M7 (T2) | Exit counters feed the recompile decision; no PIN in JIT mode |
| H32-Q4 | After PIN, does the function ever leave the pinned slot (for example a new unit load)? | A3 | No: PIN is final for the isolate |
| H32-Q5 | README §3.4 asks a Freeze table: the policy freezes at M1 exit, the budgets are tuned in A4 | M1 exit | Policy at M1 exit; budgets (4 per site, 16 per function) at the A4 gate |

## Reopens if

- SP-Facts fails a structure gate (09 §4: H30-H34).
- The RK33 trigger fires: exit counters reach their budget on JITLESS-SET (10-quality §7).
- The D17 rule picks (c) or (d) at M7 exit: the THROW or VERIFIED rows are then built.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H32, 04 §7 and 05 §10: per-mode table, jitless budgets, reserved D17 rows; confidence, SLOWPATH coverage and the budget freeze are Open questions | P3-ADR-JIT agent (M2-P3) |
| 2026-10-22 | rewrite | The jitless Rules row capped every site at its budget, but a site with no SLOWPATH variant keeps deopting until function PIN (05 §10). The row now names both budgets: a site with SLOWPATH stops deopting at 4 (est.), a site without one deopts until the function reaches 16 exits (est.) and is PINned. The diagram labels the SLOWPATH arrow with "the site has a SLOWPATH variant". Draft, before the freeze point: not counted by G8 | P3 review fix |
| 2026-10-23 | clarify | 1 `path:line` reference moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P1 integration fixer |
