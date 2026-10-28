# H32: Guard-failure policy per mode

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the per-mode table of 04 §7 and the jitless budgets of 05 §10. Nothing is built: no tier emits a guard yet |
| Fix by | Frozen M1 exit; applied A3, A4 |
| Freeze point | Parts (the Freeze table, README §3.4; H32-Q5): the policy, the counter unit and the counting rule at M1 exit; the lowering classes as an append-only list; the budget numbers at the A4 gate; the hybrid part at K-A4P; the D17 rows at the end of A5 |
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
| H32-Q1 | "HINTED: as PROFILED, at lower confidence": how confidence changes the decision (speculate or not, budget) | M1 (with H30-Q1) | Lower confidence = a higher speculation threshold; the same budgets. **Proposal 2026-10-27 (M4-P2 ADR-A0; C-07, C7):** "lower confidence" reads the per-record confidence byte of H30-Q1 (month-6 schema bump) |
| H32-Q2 | Which guard kinds get a SLOWPATH variant in optimized-AOT code, given the size cost | A4 | Shape, call-target and tag guards on hot sites only; SIZE-REPORT decides |
| H32-Q3 | Does a RUNTIME-fact guard in JIT mode also use a per-site budget, or only the T2 recompile threshold? | M7 (T2) | Exit counters feed the recompile decision; no PIN in JIT mode |
| H32-Q4 | After PIN, does the function ever leave the pinned slot (for example a new unit load)? | A3 | No: PIN is final for the isolate. **Proposal 2026-10-27 (M4-P2 ADR-A0; MODES-D14, C-07): PIN is a function state, not a code-object state.** It sets `pinned` in the function's code set, which excludes every AotOpt variant of the function (CPU levels, region and sibling, `hybrid` and `jitless` builds), also AotOpt objects that a later unit load installs; then the dispatch slot is rewritten to the next code: AotBaseline, else GenInterp, else Interp (the PIN target above). Marking one AotOpt object would not do: another Active variant would be picked before AotBaseline. In jitless mode PIN is final for the isolate, as proposed. In hybrid mode it holds until the re-tier policy lifts it (the hybrid part; the lift rule is open) |
| H32-Q5 | README §3.4 asks a Freeze table: the policy freezes at M1 exit, the budgets are tuned in A4 | M1 exit | Policy at M1 exit; budgets (4 per site, 16 per function) at the A4 gate. **Filled 2026-10-27 (M4-P2 ADR-A0; MODES-D12, MODES-D1): the Freeze table below, with the counter unit and the counting rule written now and the numbers left to the A4 gate.** Unit: one speculation exit, a guard of this code object that failed because its fact did not hold (the exit class bit of H29-Q1). Rule: a site exit counter and a function exit counter (RW, one array per code object, H29-Q5) count speculation exits only; an environment exit (a lazy deopt from a fuse pop, also from another realm, or a debugger attach) never counts, so an outside event does not punish stable code. The site budget sends a site to SLOWPATH; the function budget PINs the function (Q4). The numbers (4 per site, 16 per function, est.), and whether they become ExecPolicy defaults per platform (for example by core count) with no G8 count, are A4 calls |
| H32-Q6 | The lowering class of a code object: a unit does not record which guard lowering its code used, so a hybrid-built unit could reach a jitless isolate (the "forever deopt") | M1 exit | **Proposal 2026-10-27 (M4-P2 ADR-A0; C-07, MODES-D13):** every code object carries a 1-byte lowering class from an append-only list (Freeze table): 0 `jit` (in-memory JIT code: T1, T2, hybrid re-tier; never stored in a unit), 1 `hybrid` (hybrid units; held by the hybrid part), 2 `jitless` (jitless units). The isolate's ExecPolicy has an accept list: the JIT preset {`jit`}, hybrid {`jit`, `hybrid`, `jitless`}, jitless {`jitless`}. One filter for every code object, unit code and runtime JIT code alike: a code object whose class is not accepted is skipped, and the function keeps its next code (bytecode at worst); a unit variant that names `jit` is skipped in every preset. The byte goes into the H35 function table at A2 |

## Freeze table

README §3.4 form (H32-Q5; plan v9 §4.1 task 9; the design is [13](../plan/13-execution-modes.md) §1.4, §3.2, §3.3 and §5.4, MODES-D1). An open list works as H20 part 2: a new value is a fill when built, an existing value never changes, and G8 never counts an append. A gated part freezes at the gate that ends its build. The hybrid-only rows are one part that freezes at K-A4P; if K-A4P fails, that part is removed before its freeze point, which README §3.4 does not count.

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Policy | Decision and Rules for the JIT and jitless columns; never throw on a hint failure; the policy chosen in the IR; PIN as a function state (Q4); the counter unit and the counting rule (Q5); the lowering-class filter and the accept lists of the JIT and jitless presets (Q6) | M1 exit | Frozen-part rules (README §3.2) | SP-Facts (policy in the IR); `select()` model tests with fake code objects (month 6, C-15) |
| Lowering classes (append-only) | 0 `jit`, 2 `jitless`; value 1 `hybrid` belongs to the hybrid part (if K-A4P fails it is retired, never reused). Next by append when built: for example VERIFIED region code. An appended class names, in the same fill row, every preset whose accept list takes it; the accept lists of the Policy part take such a class by that row, with no rewrite (MC-w14-3; 13 §5.4) | None | Append only | the loader skips a variant with an unknown class: class 0xFE |
| Budget numbers (gated) | 4 per site, 16 per function (est.); whether they are per-platform ExecPolicy defaults | The A4 gate (M8 exit) | Frozen-part rules | RK33 on JITLESS-SET; A4 gates |
| Hybrid part (gated) | The hybrid column of the table; lowering `hybrid` and its acceptance (the hybrid accept list); a failed guard in hybrid code: frame to T0 + exit counter, and speculation exits past the function budget PIN the function and start the re-tier; the rule that lifts PIN in hybrid mode | K-A4P (M8 exit) | Frozen-part rules | K-A4P; mode x tier differential |
| D17 rows (gated) | The SOUND THROW row (D17 c) and the VERIFIED rows (D17 d) | The end of A5, only if the D17 rule picks them | Frozen-part rules | A5 gates |

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
| 2026-10-27 | fill (H32 is Draft) | Freeze table and Open-question text (plan v9 §4.1 tasks 2 and 9; MODES-D1, MODES-D3, MODES-D12, MODES-D13, MODES-D14, C-07): the Freeze point field names the parts; Q5 filled with the counter unit (one speculation exit) and the counting rule (environment exits never count), numbers left to the A4 gate; Q4 PIN as a function state that excludes every AotOpt variant; new Q6 the lowering class and the accept lists (JIT {jit}, hybrid {jit, hybrid, jitless}, jitless {jitless}); Q1 reads the H30-Q1 byte; a Freeze table (policy at M1 exit; lowering classes append-only; budget numbers at the A4 gate; the hybrid part at K-A4P; the D17 rows at the end of A5). The rule text of the Decision stays for Spike C (C-07). Every check keeps its result | M4-P2 ADR-A0 (VM lead role) |
| 2026-10-27 | clarify (H32 is Draft) | MODES-CHECK-w14 finding MC-w14-3 (U3; 13 §5.4): the Lowering classes row says that an appended class names, in its own fill row, every preset whose accept list takes it, so the frozen JIT and jitless accept lists (Policy part, M1 exit) can take a class named later with no G8-counted rewrite. No check, list value or freeze point changes | M4-P3 integration fixer (docs part A) |
