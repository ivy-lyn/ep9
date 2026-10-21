# H33: AOT feedback rule

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the hybrid re-tier path and the override rule. Nothing is built: no tier above T0 and no AOT code exist. K-A4P decides at A4 if hybrid mode ships |
| Fix by | Frozen M1 exit; built A3/A4 |
| Freeze point | M1 exit, the rule (README §2). Built in A3 and A4, after M1 exit, and the re-tier threshold is tuned in A4 (04 §4), so README §3.4 applies: H33-Q5 |
| Origin | R17; plan status N |
| Crates | `ep9_vm` (FeedbackVector, dispatch slots, re-tier policy), `ep9_facts` (FactSnapshot source), later `ep9_baseline` (baseline-AOT ICs write feedback), `ep9_opt` (optimized-AOT counters, instrumented T1 → T2), `ep9_unit` (RW IC cells and counters, H35) |
| Owner (plan role) | AOT senior (A3, A4; SP-Facts reopens H33, 09 §4); CORE T1 and T2 owners (the re-tier path) |
| Validated by | K-A4P (FV2 on SHORT-RUN, test262 equal) at A4; profile staleness suite (A3); mode x tier differential (A3) |
| Plan sources | [02 §6 H33](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §4](../plan/04-execution.md#4-tier-up-dispatch-slots-and-hybrid-re-tier), [05 §11](../plan/05-aot.md#11-hybrid-mode-feedback-rule-h33), [05 §9](../plan/05-aot.md#9-what-aot-compiles-d19), [H16](H16-ics-and-feedback.md), [H30](H30-ir-fact-model.md) |

## Context

- Optimized AOT code gives no feedback for free. .NET never instruments ReadyToRun code; after 30 calls it re-JITs the method as Tier1Instrumented, then Tier1 with PGO [R14][R17].
- A replayed training profile can lock in slower code: in a Leyden Havlak run the first iteration was 31% faster but the steady state was 20-40% slower [R11].
- Depends on: H16 (the shared feedback schema = FactSnapshot schema), H30 (precedence RUNTIME > PROFILED > HINTED), H14 (dispatch-slot kinds aot-baseline and aot-opt), H29 (exit counters), H32 (hybrid DEOPT → re-tier).
- Depended on by: H35 (RW IC cells and counters in units), A3 and A4, K-A4P.

## Decision

Copied from the plan row (02 §6): baseline-AOT ICs write the shared feedback schema. optimized-AOT writes only call and exit counters. Hybrid re-tiers hot optimized-AOT functions through instrumented T1, then T2. Runtime feedback overrides PROFILED and HINTED.

| Code kind (05 §11) | Writes | Hybrid re-tier path |
|---|---|---|
| T0, T1 | full FeedbackVector | T0 -> T1 -> T2 by v1 thresholds |
| baseline-AOT | full feedback schema through its RW IC cells (H16) | to T2 when hot |
| optimized-AOT | call and exit counters only | hot -> instrumented T1 -> T2 |

```text
 aot-opt --calls >= 1,000 (est.)--> T1 instrumented --T2 threshold--> T2 (RUNTIME facts)
    |                                     ^
    +-- guard fails -> DEOPT -> T0 frame -+   (threshold owned by 04, tuned in A4)
```

| Event (04 §4) | JIT mode | hybrid mode | AOT-jitless mode |
|---|---|---|---|
| Function gets hot | T0 -> T1 -> T2 by thresholds | aot-opt -> instrumented T1 -> T2; aot-baseline -> T2 (its ICs wrote feedback) | Never re-tiers |

- Runtime feedback overrides PROFILED and HINTED facts: the H30 merge rule, applied per fact kind (05 §11).
- A function whose source hash differs from the profile ignores the profile (05 §9).
- K-A4P decides if hybrid ships; on failure v1 ships no hybrid mode (05 §11).

**As built (e719ffa).** The FactSnapshot source byte `Runtime` is documented as "IC and feedback data of this run (T0, T1, baseline-AOT ICs)" (`crates/ep9_facts/src/bits.rs:125-126`). H16 states that baseline-AOT ICs write the same schema (`docs/adr/H16-ics-and-feedback.md:16`). The dispatch-slot kinds `AotBaseline = 3` and `AotOpt = 4` exist (`crates/ep9_vm/src/dispatch.rs:28-31`). T0 writes the lazy FeedbackVector (H16); no other tier exists.

## Rules

| Rule | Check | From |
|---|---|---|
| Baseline-AOT ICs write the shared feedback schema (H16), so a hot aot-baseline function can go to T2 on RUNTIME facts | M1: Spike C / SP-Facts (one schema: a kernel from a FactSnapshot file); A3 unit tests: a baseline-AOT run exports a RUNTIME FactSnapshot that `ep9_facts` validates (H16 `tests/validate.rs`) | M1; A3 |
| Optimized-AOT code writes only call and exit counters, in RW data | M1: Spike C / SP-Facts; unit version checks see the counters in the RW template (10-quality §2, H35) | M1; A4 |
| In hybrid mode a hot optimized-AOT function re-tiers through instrumented T1, then T2; a jitless build never re-tiers | mode x tier differential (10-quality §4); K-A4P (FV2 on SHORT-RUN: wall time ≤ 0.9x node, time-to-peak ≥ 15% better than ep9 JIT-only, steady state within 2%, test262 equal; 10-quality §3) | A3; A4 |
| RUNTIME facts override PROFILED and HINTED facts of the same kind | M1: Spike C / SP-Facts (H30 merge rule); profile staleness suite: stale functions ignore the profile (10-quality §2) | M1; A3 |
| A profile whose source hash differs from the function is ignored | profile staleness suite (10-quality §2) | A3 |
| Instrumented T1 code is made only for hot AOT functions | K-A4P and the RK44 trigger (FV2 time-to-peak or steady-state miss; 10-quality §7) | A4 |

## Consequences

- **Gains.** A training profile cannot lock in slow code in hybrid mode: the JIT re-tiers on fresh feedback. Baseline-AOT code feeds T2 like T1 code does.
- **Costs.** Instrumented T1 code for hot AOT functions, compile CPU for re-tiered code, and slower code while instrumented (05 §11). AOT-jitless mode never adapts after load.
- **Follow-ups.** The re-tier threshold (1,000 calls, est.) is tuned in A4. K-A4P at A4.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R17] | .NET: "we never instrument R2R'd code"; R2R code is promoted after 30 calls to Tier1Instrumented | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| [R14] | .NET treats R2R code as Tier0 (optimized, not instrumented); re-JIT as Tier1Instrumented, then Tier1 with PGO | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R11] | Leyden Havlak: replayed profile, first iteration -31%, steady state 20-40% slower; AOT as a tier, not the top tier, in JIT mode | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |

## Validation

- M1: SP-Facts (a failed gate reopens H30-H34, 09 §4). A3: profile staleness and mode x tier differential. A4: K-A4P on SHORT-RUN; on failure v1 ships no hybrid mode.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H33-Q1 | Where optimized-AOT keeps its call counter (unit RW template, H35, or the FunctionInfo), and how the threshold check runs | A4 | One RW counter per function in the unit, checked at function entry beside the interrupt poll |
| H33-Q2 | Does instrumented T1 start from an empty FeedbackVector, or from the PROFILED facts the AOT code used? | A4 | Empty vector; PROFILED facts stay as lower-precedence inputs (H30) |
| H33-Q3 | How baseline-AOT RW IC cells (H35) map to the FeedbackVector that T2 reads (one store or a copy at tier-up) | A3 | The FeedbackVector is the store; unit IC cells point into it |
| H33-Q4 | Exit counters of optimized-AOT code in hybrid mode: do they also trigger the re-tier, or only the call counter? | A4 | Both: exits past the H32 budget also start the instrumented re-tier |
| H33-Q5 | README §3.4 asks a Freeze table: the rule freezes at M1 exit, the threshold is tuned in A4 | M1 exit | Rule at M1 exit; the threshold at the A4 gate |

## Reopens if

- K-A4P fails at A4: hybrid mode does not ship, and the hybrid rows go away (a rewrite).
- SP-Facts fails a structure gate (09 §4).
- The RK37 trigger fires: FV2 steady state > 2% worse (training-profile lock-in; 10-quality §7).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H33, 04 §4 and 05 §11: what each code kind writes, the hybrid re-tier path and the override rule; counter homes, the instrumented start state and the threshold freeze are Open questions | P3-ADR-JIT agent (M2-P3) |
