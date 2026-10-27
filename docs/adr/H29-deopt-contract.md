# H29: Profiling and deopt contract; position-independent metadata

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the Spike B data for the position part, and the open deopt-exit parts. Reopened 2026-10-22: gate window 2 put JIT frames on the VM stack (Spike B placement (b), H14-Q5). No deoptimizer exists yet; Spike C and SP-Facts (M1) validate the exit parts |
| Fix by | Draft M0 month 3; Spike C + SP-Facts; frozen M1 exit |
| Freeze point | Parts (the Freeze table, README §3.4; H29-Q8): the contract at M1 exit ([09 §2](../plan/09-milestones.md) M1 exit item 9); three append-only lists with no close point; the parts built in A4 at the A4 gates |
| Origin | v1#27 + R16-D4; plan status E |
| Crates | `ep9_vm` (offset slot, deoptimizer runtime, exit counters, interrupt polls); later `ep9_opt` (deopt-metadata emission), `ep9_masm` (`DeoptExit` relocation, H34), `ep9_unit` (deopt translations in rodata, H35) |
| Owner (plan role) | JIT/platform lead (Spike C); AOT senior (SP-Facts: PI metadata) ([09 §4](../plan/09-milestones.md#4-spikes)) |
| Validated by | Position part: SPIKEB(structural) and SPIKEB(b) check (c) (Spike B, M0 month 2). Exit parts: Spike C + SP-Facts (M1) |
| Plan sources | [02 §6 H29](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §9](../plan/04-execution.md#9-deopt-contract-h29), [04 §5 spikes](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [05 §10](../plan/05-aot.md#10-jitless-guard-policy-no-deopt-loops), [H14](H14-frames-and-dispatch.md), [H24](H24-debug-and-profiling-hooks.md) |

## Context

- T2 is the tier that matters for speed (G4), and T2 code speculates. A failed guard must rebuild T0 frames. AOT text is read-only and may be signed (iOS), so the metadata can hold no absolute address and a deopt can patch no code (02 §6).
- TurboFan code has 12.5-17.1 deopt points per KB [R14]. So this format sets much of the unit size.
- Depends on: H2 (bytecode offset = the only deopt and OSR id), H14 (canonical frame, offset slot, dispatch slots, pc→code registry), H16 (feedback), H23 (interrupt polls), H24 (debugger deopt), H31 (fuse pops cause lazy deopt).
- Depended on by: H30 (the graph builder emits exits), H32 (DEOPT and PIN paths), H33 (exit counters), H34 (`DeoptExit` relocation), H35 (rodata deopt translations, RW exit counters), H10 (a moving nursery would use these maps).

## Decision

Copied from the plan row (02 §6): v1 content: slot kinds, constness and watchpoints, per-exit bytecode offset + live-value locations, MovHint-style rebuild, per-bytecode exit counters, code dependencies. Plus per exit: pc offset from code start, value locations (reg/stack/const-index/materialize), inline frame chain. One shared deopt entry. Lazy deopt by return-address swap only. Every interrupt poll (loop back-edge, function entry) is a lazy-deopt point with full metadata. Exit counters in RW data. Deoptimizer runtime in `ep9_vm`.

| Per-exit field (04 §9) | Content |
|---|---|
| pc offset | From the code start, so the metadata is position-independent |
| Bytecode offset | The only deopt and OSR identity (H2) |
| Value locations | Every live register and the accumulator: reg, stack, const-index or materialize |
| Inline frame chain | Function id + bytecode offset for each inlined frame |
| Exit counter | Index of a counter in RW data |

```text
eager: Guard fails -> DeoptExit(id) -> shared deopt entry (ep9_vm)
         -> pc->code registry -> metadata at pc offset -> rebuild T0 frames (MovHint style, inline chain)
         -> bump RW exit counter -> resume T0 at the bytecode offset
lazy:  fuse pops -> mark dependent code -> swap return addresses on the stack -> frames enter the deopt entry on return
```

**The position part as built (Spike B, feature `spike-native` only).** Placement (b) is the D21 placement since gate window 2 (H14-Q5): (b) frames live on the VM register stack. Placement (a) (native-stack frames) failed the D21 rule in gate window 1 and stays as spike code. In both, a JIT frame is a canonical VM frame (H14 JIT frames paragraph, (a) and (b), `docs/adr/H14-frames-and-dispatch.md:65-66`). JIT code never hands the runtime a native pc:

| Fact | Where (e719ffa; the (b) refs at c39f782) |
|---|---|
| Every runtime-table call from JIT code passes `(cx, fp, bytecode offset, values..)` | `crates/ep9_vm/src/native.rs:10-11`; the table fn types `RtFn0`-`RtFn3` `crates/ep9_vm/src/native.rs:97-109` |
| Rules "records the state first" and "a bytecode offset, not a pc" (the same code works for JIT code and unit text) | `crates/ep9_vm/src/native.rs:21-22` |
| `native::enter` publishes fp and saves the offset through `Runtime::enter` before the table fn body runs | `crates/ep9_vm/src/native.rs:556-573`; `crates/ep9_vm/src/runtime.rs:385-399` (offset slot fp-1, `crates/ep9_vm/src/frame.rs:68`) |
| The a64 emitter loads the offset as an immediate (w2) for each table call | (a): `spikes/spike-b/src/jit/a64.rs:90-92`; (b): `spikes/spike-b/src/jit_b/a64.rs:107-113` |
| (b) only: a table result of 0 runs `RuntimeTable::unwind(cx, fp, off)`, which unwinds from that bytecode offset (`error::unwind`), sets `Cx::out_fp`/`out_pc`, and the (b) code tail-jumps there | `spikes/spike-b/src/jit_b/a64.rs:16-18`; `crates/ep9_vm/src/native.rs:142-148` |
| Spike B code has no absolute address and no relocation; each emitted bytecode op records its native offset from the code start | `spikes/spike-b/src/emit.rs:139-146` (`FunctionCode.code`, `native_offsets`) |
| No deopt metadata exists: `DEOPT_META_VERSION` = 0. The unit header carries it (offset 112); a mismatch is the BYTECODE outcome (H35 S4 step 4) | `crates/ep9_vm/src/versions.rs:29-30`; `crates/ep9_unit/src/header.rs:115-116`; `docs/adr/H35-aot-unit-format.md:55` |
| `JumpLoop` is documented as the interrupt poll and H29 lazy-deopt point | `crates/ep9_bytecode/src/opcodes.rs:437` |

## Rules

| Rule | Check | From |
|---|---|---|
| JIT code passes a bytecode offset, never a native pc, to every runtime call and poll; the callee writes the frame's offset slot first | SPIKEB(structural) check (c): an exception crosses a JIT frame and Error.stack equals T0's, `at g (defs.js:3:10)` (`spikes/spike-b/src/structural.rs:9`; H24 rule row `docs/adr/H24-debug-and-profiling-hooks.md:42`). For (b): SPIKEB(b) check (c), a T0 throw under (b) code and a throw from a (b) runtime call cross the (b) frame to a T0 catch at the right line, and Error.stack equals T0's (`spikes/spike-b/src/jit_b/structural.rs:11`) | Spike B (M0 month 2); (b): M2-P4 |
| The position of a JIT frame below the top comes from its offset slot, as for T0 | SPIKEB(structural) checks (b) and (c); `tests::native::exceptions_cross_native_frames`; `tests::exceptions::*` for T0 frames | Spike B (M0 month 2) |
| Deopt metadata is keyed by the pc offset from the code start; it holds no absolute address | M1: Spike C / SP-Facts (SP-Facts gate: "PI deopt metadata", 09 §4) | M1 |
| The bytecode offset is the only deopt and OSR id; one eager deopt from a native frame resumes T0 at that offset | M1: Spike C / SP-Facts (deopt correct under the tier-differential test) | M1 |
| One shared deopt entry rebuilds T0 frames from the value locations and the inline frame chain | M1: Spike C / SP-Facts; later the "Tier differential + deopt" fuzz row (10-quality §4) | M1; fuzz from M5 |
| Lazy deopt swaps return addresses only; it never patches code | M1: Spike C / SP-Facts; from A2 unit text is RX-only (H35), so a code patch would fault in the unit tests | M1; A2 |
| Every interrupt poll in T1, T2 and AOT code is a lazy-deopt point with full metadata | M1: Spike C / SP-Facts; the inspector tests of H24 (session connect deopts all frames) | M1; M4 |
| Exit counters live in RW data, never in code | M1: Spike C / SP-Facts; unit version checks (10-quality §2) see them in the RW template (H35) | M1; A2 |
| The deoptimizer runtime is in `ep9_vm`; `ep9_opt` has none; the jitless build keeps the deoptimizer | `cargo xtask feature-matrix` builds `aot` without `jit` (H27); a dep-guards row for `ep9_opt` when the crate is added | M1 (crate added) |

## Consequences

- **Gains.** One deopt path serves JIT code, unit code and the debugger. Unit text stays read-only and signable. Positions already work through JIT frames with one store per runtime call (Spike B).
- **Costs.** Metadata at every exit and every poll; with 12.5-17.1 exits per KB [R14] it is a large share of unit size. Lazy deopt needs every dependent frame to return through the entry.
- **Follow-ups.** Spike C + SP-Facts (M1) fill H29-Q1 to H29-Q5. T0 records the H29 profiles in M1 (09 §2 M1 scope); M2 exit item 10 runs gates 1-3 with them on.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | Deopt to interpreter frames from per-exit metadata (bytecode offset plus each live register and the accumulator); MovHint-style lazy rebuild; HTC4: slot kinds plus exit flags | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R07] | Maglev was built on top of the existing TurboFan deopt infrastructure | [../research/07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R14] | V8 lazy deopt marks code and patches the return address, not the code; TurboFan code has 12.5-17.1 deopt points per KB | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R16] | D4: per exit the pc offset and each live value's location; one shared entry; exit counters in RW data | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| Spike B | JIT frames are canonical VM frames; table functions take a bytecode offset; check (c) passes on 4 CI runners (run 39677340839, 5c9034a) | [H14](H14-frames-and-dispatch.md), [H24](H24-debug-and-profiling-hooks.md) |

## Validation

- M0 month 2: SPIKEB(structural) (c) proves the position part only: "bytecode offset = only deopt/OSR id" (02 §6 H2) holds for positions.
- M1: Spike C (one eager deopt, native frame to VM-stack frame, tier-differential test) and SP-Facts (PI metadata). A failed gate reopens H29 (04 §5).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H29-Q1 | Per-exit record format: how the pc offset, the bytecode offset and the inline chain are encoded, and the size per exit | M1 (Spike C + SP-Facts) | A per-function table sorted by pc offset; report bytes per exit against [R14]'s 12.5-17.1 exits per KB. **Note 2026-10-27 (M4-P2 ADR-A0; C-04): each exit record carries an exit class bit {speculation, environment}.** A speculation exit is a guard whose fact did not hold; an environment exit is a lazy deopt from an outside event (a fuse pop, also from another realm; a debugger attach). Only speculation exits count toward the H32 budgets and PIN (H32-Q5). As a note, not a rule: the record names the feedback slot of its guard, so a recompile can see why it failed (MC-w13-7). The record tags, location tags and inline frame kinds are append-only lists (Freeze table); the byte format stays with Spike C |
| H29-Q2 | Value-location kinds and the "materialize" form (objects removed by escape analysis) | M1 (Spike C + SP-Facts) | reg, stack, const-index (unit constant pool), materialize with a field list; Spike C covers reg, stack and const-index |
| H29-Q3 | The shared deopt entry: register-save layout per ABI (aapcs64, sysv, win64) and how it finds the metadata (pc→code registry, H14) | M1 (arm64, Spike C); x86-64 before the first x86-64 tier that deopts | One entry per ABI in `ep9_vm`; lookup by the registry, then the pc offset. Note (2026-10-22, placement (b)): a (b) frame is on the VM stack, but a (b) → callee call still nests one native level through `RuntimeTable::call_any` (H14), so the entry must also rebuild a frame that is below such a level |
| H29-Q4 | Lazy deopt: when the return-address swap runs, how CFI unwinding and the native stack walk (H24 R1) see a swapped frame, and whether it works with hardware shadow stacks (a shadow stack checks each return address) | M1 (Spike C) | Swap at the fuse pop for every dependent frame on the stack; check the unwinder through a swapped frame in Spike C. Note (2026-10-22, placement (b)): a (b) frame returns by a tail jump to `cx.table[*pc]` from its VM return pc (fp-7), and a (b) → callee call nests one native level through `call_any`; the swap must handle both the VM return pc and that native level |
| H29-Q5 | Exit-counter home for JIT code: H16 has no exit slot kind, and "RW data" is the unit's RW template (H35) | M1 (H16 freeze) | Per-site counters in an RW array per compiled function; a FactSnapshot exit-flag kind needs a `FACT_SCHEMA_VERSION` bump. **Proposal 2026-10-27 (M4-P2 ADR-A0; C-04, F1):** one RW counter array per code object (JIT memory or the unit RW template, H35); the exit record holds the counter index. The exit flag goes into the one month-6 `FACT_SCHEMA_VERSION` bump (V9-D10) |
| H29-Q6 | The OSR entry: how a T0 frame enters optimized code at a loop back-edge, given the bytecode offset as the id | M1 exit (freeze) | Same metadata in reverse: value locations at the loop header's offset. **Proposal 2026-10-27 (M4-P2 ADR-A0; C-04, F3, F4):** OSR out of AOT code = a lazy deopt at the `JumpLoop` poll (every poll is a lazy-deopt point; this mechanism is the Contract part). What the T0 frame does next is mode policy, not contract: in hybrid mode it tiers up as in JIT mode (the H33 Hybrid part; [13](../plan/13-execution-modes.md) T8 is n/a in JIT and jitless mode). OSR into AOT code is reserved: the record tag osr-entry is the next value of the record-tag list, and its format is a gated part (Freeze table) |
| H29-Q7 | A GC point inside the deopt entry (materialize allocates) while T0 frames are half-built | M1 (Spike C) | Build the frames in a scratch area that the GC scans precisely; push them after the last allocation |
| H29-Q8 | README §3.4 asks a Freeze table for a row built after M1 exit. Does any part (counter budgets, H32) need a later freeze point? | M1 exit | The whole row freezes at M1 exit; tuning values live in H32 and 05 §10. **Filled 2026-10-27 (M4-P2 ADR-A0; MODES-D1, C-04): the Freeze table below.** Tuning values stay in H32. Three lists stay open for appends, and the A4-built parts freeze at the A4 gates, the gate that ends their build |

## Freeze table

README §3.4 form (H29-Q8; plan v9 §4.1 task 9; the design is [13](../plan/13-execution-modes.md) §3.6 and §5.4, MODES-D1). An open list works as H20 part 2: a new value is a fill when its kind is built, an existing value never changes, and G8 never counts an append. A negative test probes an unknown value at the top of the range (tag 15), never the next free one, so an append keeps every test result.

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| Contract | Decision and Rules: the per-exit fields, one shared entry, lazy deopt by return-address swap only, every poll a lazy-deopt point, RW exit counters, the deoptimizer in `ep9_vm`, position-independent metadata; the fills of Q1-Q7 that Spike C and SP-Facts make; the exit class bit (Q1) | M1 exit | Frozen-part rules (README §3.2) | Spike C and SP-Facts; the tier-differential test |
| Record tags (append-only) | v1: exit, poll, call (a call return point: a lazy deopt lands there, so every call site of optimized code needs a record with value locations). Next by append: osr-entry | None | Append only (a fill when built) | the deopt reader refuses a code object with an unknown tag, and the function runs bytecode: tag 15 |
| Value-location tags (append-only) | v1: 0 reg, 1 stack, 2 const-index (Spike C builds these three). Next by append: materialize, duplicate | None | Append only | unknown tag -> code object refused: tag 15 |
| Inline frame kinds (append-only) | v1: bytecode frame (function = an index into a per-code FunctionRef table + bytecode offset). Next by append: builtin continuation, construct stub | None | Append only | unknown kind -> code object refused |
| A4-built (gated) | Resume in non-T0 code ("T0 at the bytecode offset, or resume-capable code of the same function"); the osr-entry record format; the materialize payload | The A4 gates (M8 exit) | Frozen-part rules | A4 gates; mode x tier differential |

## Reopens if

- Spike C or SP-Facts fails a correctness or structure gate (04 §5: reopens H2, H14, H16, H17 and H29-H34 before M1 exit).
- ~~The D21 verdict moves JIT frames to the VM stack (H14-Q5, placement (b)): the deopt entry then rebuilds frames in place.~~ Fired 2026-10-22 (gate window 2, M2-W2-D21-B): placement (b) passed. The deopt entry rebuilds (b) frames in place on the VM stack; the `call_any` level is noted at H29-Q3 and H29-Q4.
- SPIKEB(structural) check (c) fails, or a JIT frame's position needs a native pc.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H29 and 04 §9, with the Spike B offset-slot data as evidence for the position part; the deopt-exit parts are Open questions for Spike C + SP-Facts | P3-ADR-JIT agent (M2-P3) |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | reopen | The "Reopens if" row on the D21 verdict fired: gate window 2 gave pass for placement (b) (RT 14.618 [13.999, 15.006], c39f782; M2-W2-D21-B), so JIT frames live on the VM stack. The row is marked fired; H29-Q3 and H29-Q4 note that (b) → callee calls still nest one native level through `call_any`, and that a (b) frame returns through its VM return pc, so lazy deopt must handle both (M2-P5-PLAN-D16) | P5-DOCS-SYNC agent (M2-P5) |
| 2026-10-22 | fill | The position part names placement (b): the Decision paragraph, two Facts rows (the (b) offset immediate, `RuntimeTable::unwind`), and SPIKEB(b) check (c) (`jit_b/structural.rs:11`) next to the (a) check (c) in Rules row 1 and in Validated by. Every existing check keeps its result (M2-P5-PLAN-D16) | P5-DOCS-SYNC agent (M2-P5) |
| 2026-10-23 | clarify | 3 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). The H14 ref of the position part still names the two JIT-frame paragraphs ((a), then (b)); both still say a JIT frame is a canonical VM frame, also for x86-64 (b) since M3-P0, so the claim holds (a pin in the drift script: the (b) paragraph text changed). No check changes | M3-P0 integration fixer |
| 2026-10-23 | clarify | 5 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P1 integration fixer |
| 2026-10-25 | clarify | 1 `path:line` reference moved to the code after the M3-P3 fix round 2 change (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `Runtime` got the `id` field (the key of the isolate's module hosts). The ref names the same item at the new line. No check changes | M3-P3 fix round 2 (import() and TLA) |
| 2026-10-26 | clarify | 2 `path:line` references moved to the code after the M4-P0 merge (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `runtime.rs` 336-350 -> 385-399 (the interpreter-state record before a GC point) and `versions.rs` 20-21 -> 29-30 (`DEOPT_META_VERSION`) (P0-VM4: the month-4 `Runtime` fields and `rng_seed`, +4 lines from the per-isolate `rng_seed(id)` of the Codex r1 vm fix; the OBJECT_LAYOUT 4 and IC_LAYOUT 2 docs in `versions.rs`). Text only; no check changes | M4-P0 integration fixer |
| 2026-10-27 | fill (H29 is Draft) | Freeze table and Open-question text (plan v9 §4.1 tasks 2 and 9; MODES-D1, MODES-D3, C-04): the Freeze point field names the parts; a Freeze table (the contract at M1 exit; record tags, value-location tags and inline frame kinds as append-only lists; the A4-built resume, osr-entry and materialize part at the A4 gates); Q1 names the exit class bit {speculation, environment} and, as a note, the guard's feedback slot; Q5 one RW counter array per code object; Q6 OSR out of AOT at the poll, into AOT reserved; Q8 filled. Q1, Q2 and Q4 formats stay with Spike C. Every check keeps its result | M4-P2 ADR-A0 (VM lead role) |
| 2026-10-27 | clarify (H29 is Draft) | Review fix from the M4-P2 Claude reviewer (ADR-A0 scope, F7): the H29-Q6 proposal put the hybrid-only tier-up after OSR out of AOT code into the Contract part (frozen at M1 exit); the Contract keeps the mechanism (a lazy deopt at the `JumpLoop` poll) and the tier-up is the H33 Hybrid part's. No check changes | M4-P2 integration fixer B |
| 2026-10-27 | clarify (H29 is Draft) | The Facts row "Every runtime-table call from JIT code passes `(cx, fp, bytecode offset, values..)`" named the table fn types `RtFn0`-`RtCompareFn` at `native.rs:97-106`; M4-P3 KEYED (8140b68) added `RtFn3` (three value operands, for `set_keyed`) after them. The row now names `RtFn0`-`RtFn3` at `native.rs:97-109` (M4-P3 review K-2). The ADRLINE run of fixer part B (after its code fixes) adds 2 `path:line` moves to this row: `crates/ep9_vm/src/native.rs` 467-484 -> 556-573 and 139-145 -> 142-148 (KEYED and the P3 fixer moved the code), each with the same line text. Text only; no check changes | M4-P3 integration fixer (part B), from the M4-P3 review (K-2) |
