# H34: One Rust backend with abstract relocations

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row and the Spike B emitter evidence (JIT code holds no absolute address and no relocation; a test checks it for placement (b) on AArch64 and on x86-64; x86-64 (b) is built, and its x86-64 runs are in the M3-P0 sandbox and CI). No `ep9_masm` exists, so no relocation kind is built. M3-P1 SP-UNIT built unit text with 0 relocations (`spike-b unit`, `gate.sh unit`) and filled H34-Q1 to H34-Q3; H34-Q4 and H34-Q5 stay open |
| Fix by | ADR frozen M1 exit; built A2/M5 |
| Freeze point | M1 exit, the whole row (README §2). The kinds are built after M1 exit (A2 unit finalize, M5 JIT finalize), so README §3.4 applies: H34-Q5 |
| Origin | R16-D10 (refines N5); plan status N |
| Crates | `ep9_masm` (the instruction stream, the relocation kinds, `finalize_jit` and `finalize_unit`; not created yet), `ep9_unit` (the writer rejects HeapConst; zero text relocations, H35); today the throwaway emitters of `spikes/spike-b` |
| Owner (plan role) | JIT/platform lead (Spike B emitters, `ep9_masm`); AOT senior (SP-Unit, unit finalize) ([09 §4](../plan/09-milestones.md#4-spikes)) |
| Validated by | Spike B: the jit_b test `b_code_has_no_ret_and_no_address` (AArch64 (b) code). SP-Unit (M3-P1): a unit-text scan finds 0 relocations. A2 gate: zero text relocations in every CI unit ([10 §5](../plan/10-quality.md#5-other-ci-checks)) |
| Plan sources | [02 §6 H34](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [05 §8](../plan/05-aot.md#8-aot-unit-format-h35), [08 §2](../plan/08-repo.md#2-crates), [H35](H35-aot-unit-format.md), [H29](H29-deopt-contract.md) |

## Context

- JIT code and AOT unit code must share the deopt and frame formats (H14, H29). Two backends would build them twice (02 §6).
- AOT unit text is read-only, may be signed, and is mapped once per process. So it can hold no absolute address and no text relocation (H35, H29). Wasmtime refuses precompiled code with relocations outside `.debug_*` [R14].
- Depends on: H7 (the code range and direct-call reach, H07-Q2), H13 (RootIndex: stable indices), H14 (frames, dispatch slots), H29 (DeoptExit, pc-offset metadata), H30 (the IR has no raw heap constants), H35 (unit container, link table, RW template, H35-Q9 unit-data register).
- Depended on by: H35 (the unit writer and loader), H29 (the DeoptExit kind), T1 in M5 ("T1 emits through the H34 relocation kinds", 09 §2), A2 (unit finalize).

## Decision

Copied from the plan row (02 §6): `ep9_masm` emits JIT code or unit code from one instruction stream. Kinds: Local, Stub, Runtime, LinkSlot, IcCell, RootIndex, DeoptExit, HeapConst (JIT only). No LLVM. Cranelift only for the optional T3.

```text
ep9_baseline / ep9_opt --one instruction stream + abstract relocations--> ep9_masm
  ep9_masm --finalize_jit (`jit`)--> W^X code space; absolute addresses allowed; dispatch-slot write; jitdump
  ep9_masm --finalize_unit (`aot`)--> unit text (RX, zero text relocations) + RW link table + RW IC cells
                                      + PI deopt metadata --> ep9_unit writer (.o / .ep9u)
```

| Kind (04 §5) | Meaning | JIT finalize | AOT finalize |
|---|---|---|---|
| Local | Label or branch inside the function or unit | pc-relative, resolved at emit | pc-relative, resolved at emit |
| Stub(id) | Shared IC stub or trampoline | Direct call inside the code range | pc-relative call if the stub is in the unit; else a call through a link slot |
| Runtime(fn) | Rust slow path or builtin (H20) | Direct or indirect call | Indirect call through the isolate table (isolate register) |
| LinkSlot(i) | External reference resolved at load or first run | Resolved at finalize; an immediate is allowed | A load of RW link cell i through the unit-data register x27/r14 (H34-Q1, H35-Q9) |
| IcCell(i) | IC or feedback cell | Address of the FeedbackVector slot | A load of RW IC cell i through the unit-data register x27/r14 (H34-Q1, H35-Q9) |
| RootIndex(n) | Isolate root, intrinsic or atom by stable index (H13) | Load from the isolate root table | Load from the isolate root table (isolate register) |
| DeoptExit(id) | Exit to the shared deopt entry | Call the shared entry; metadata keyed by pc offset | Same; metadata keyed by pc offset |
| HeapConst(i) | Raw heap pointer | Embedded, held weakly with a dependency | **Rejected by the unit writer** |

- 04 §5 wrote the AOT form of LinkSlot and IcCell as "pc-relative load". That reaches one RW copy only, because unit text is mapped once per process and each isolate has its own RW copy (H35-Q9, M2-JIT-ABI). H34-Q1 is filled: a load through the unit-data register, and 04 §5 now says so (M3-P1 SP-UNIT).
- The AOT finalizer gives zero text relocations; an xtask check enforces it (04 §5). The IR has no raw heap constants (H30), so the HeapConst rejection is a backstop.

**As built (Spike B, M0 month 2; spike code, not `ep9_masm`).** The Spike B emitters (`spikes/spike-b/src/emit.rs`, frozen at the P2 merge) state the zero-relocation rule for every function they emit: `FunctionCode::code` has no absolute address and no relocation; branches are pc-relative inside the function, and runtime functions are reached through `Cx::runtime_table` behind the isolate register (`spikes/spike-b/src/emit.rs:19`). Immediates are constants (Value tags, offsets, compare codes), never addresses (`spikes/spike-b/src/jit_b/a64.rs:66-67`).

| Emitter | Placement, target | Check |
|---|---|---|
| `jit_b` (P4-PLACEMENT-B; x86-64 M3-P0 P0-B-X64) | (b), AArch64 and x86-64 | `jit_b::tests::b_code_has_no_ret_and_no_address` (`spikes/spike-b/src/jit_b/mod.rs:286-314`): the same function from two isolates gives the same bytes on both architectures, and the AArch64 code has no `ret` (it leaves by `br x16` only). x86-64: `jit_b::tests::x86_64_b_code_leaves_by_the_exit_jump_only` (no `ret` at a reported instruction start; one exit jump) and `win-static.sh` (`llvm-objdump -d`: no `ret`, one indirect jump) |
| `jit` (P2-SPIKE-B) | (a), AArch64 and x86-64 | No address by emitter design; no test. (a) failed the D21 rule in gate window 1 |
| (b) on x86-64 | built (M3-P0 P0-B-X64, M2-P5-PLAN-D25; `spikes/spike-b/src/jit_b/x64.rs`) | The `jit_b` row above: the two-isolate test covers it. Its only absolute-looking operands are constants (Value tags, `Cx` and table offsets); branches are rel8/rel32 inside the function |

## Rules

| Rule | Check | From |
|---|---|---|
| Spike B (b) code holds no absolute address and no relocation; the same function gives the same bytes in two isolates | `cargo test` in `spikes/spike-b`: `jit_b::tests::b_code_has_no_ret_and_no_address` (AArch64 and x86-64 code) | Spike B (M2-P4); x86-64 M3-P0 |
| Spike B (a) code holds no absolute address | No address by emitter design (`emit.rs:19`); no test | Spike B (M2-P2) |
| Unit text has 0 relocations | SP-Unit: a text scan finds 0 absolute relocations; from A2 the xtask "zero text relocations" check on every CI unit (10-quality §5) | M3-P1; A2 |
| The unit writer rejects HeapConst | A2: the "Zero text relocations" check ([08 §7](../plan/08-repo.md); [10 §5](../plan/10-quality.md#5-other-ci-checks)): the writer rejects HeapConst | A2 |
| JIT and unit code come from one instruction stream; no LLVM in JIT or AOT code | Not built: `ep9_masm` does not exist yet. A dep-guards row for `ep9_masm` when the crate is added ([08 §7](../plan/08-repo.md)) | M5 (crate added) |

## Consequences

- **Gains.** Deopt and frame formats exist once, for JIT and unit code (02 §6). Code without addresses can be placed in a JIT chunk or in unit text unchanged; Spike B code already has that property, so SP-Unit can place it behind the frozen `Finalize` trait (`spikes/spike-b/src/emit.rs:166-175`).
- **Costs.** Every external reference in unit code costs a load through a register or a table (isolate table, root table, link or IC cell). LLVM-level code quality is ruled out for T1 and T2 (no LLVM; Cranelift only for T3).
- **Follow-ups.** M3-P1 SP-UNIT owns H34-Q1 to H34-Q4 (plan-v7 §14). `ep9_masm` is created for T1 (M5) and the unit finalizer for A2.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R06] | LLVM took > 75% of FTL compile time; Cranelift has no deopt value maps | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R14] | One backend for JIT and AOT; Wasmtime refuses precompiled code with text relocations; position-independent, isolate-independent code through a root register (V8 embedded builtins) | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R16] | D10: one Rust backend for JIT and AOT, with abstract relocation kinds | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| Spike B | The (b) emitter rule and its test, on AArch64 and x86-64; the (a) emitters by design only | `spikes/spike-b/src/emit.rs`, `spikes/spike-b/src/jit_b/mod.rs` |

## Validation

- Spike B (M0 month 2): the jit_b test above, on every `spike-b` test run (it emits AArch64 code on any host).
- SP-Unit (M3-P1; moved by M2-D21-FAIL, built on placement (b)): unit text with 0 relocations, LinkSlot and IcCell through the unit-data register, load, call, unwind, GC scan and fallback on 4 targets. A failed SP-Unit gate reopens H34 and H35 (09 §4).
- Spike C + SP-Facts (M1): a failed gate reopens H29-H34 (04 §5). A2 gate: zero text relocations in every CI unit.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H34-Q1 | How does unit text reach a LinkSlot or IcCell cell in its isolate's RW copy? 04 §5 says pc-relative, which reaches one copy only | M3-P1 SP-UNIT | **Filled 2026-10-23 (M3-P1 SP-UNIT): as proposed.** A register-relative load `[x27 + 8 * i]` / `[r14 + 8 * i]` from the unit-data register (H35-Q9), which holds the base of the isolate's RW copy: external references, link cells (strong root range), IC cells (weak root range), counters. Built in `spikes/spike-b/src/unit/` (`a64.rs`, `x64.rs`, `emit.rs`); `spike-b unit` checks (c)-(e) |
| H34-Q2 | Which relocation forms may unit text keep: pc-relative branches inside the unit only, or also pc-relative loads of unit rodata? | M3-P1 SP-UNIT | **Filled 2026-10-23 (M3-P1 SP-UNIT).** The spike unit text keeps only pc-relative branches inside one function: no `adrp`, no `bl`, no pc-relative load. The text scan (`spikes/spike-b/src/unit/scan.rs`) rejects the other forms on AArch64, and finds 0 relocations in the AArch64 and x86-64 text. A pc-relative load of unit rodata stays allowed (rodata is one copy per process), but SP-Unit did not need one; A2 builds and checks it with real rodata. Every other reference goes through a register or a table |
| H34-Q3 | Calls beyond direct-call reach (H07-Q2, `docs/adr/H07-address-space-and-code-range.md:88`): a veneer or an indirect call? | M3-P1 SP-UNIT (unit text); M5 (JIT code range) | **Filled 2026-10-23 for unit text (M3-P1 SP-UNIT): an indirect call through a register.** Unit text has no direct call out of a function (the scan rejects `bl`). A runtime call loads the runtime table from RW external-reference word 0, then the entry, and calls through a register (`blr x16`; x86-64 `call [rax + entry]`). No veneer was needed. The JIT code range part stays open for M5 |
| H34-Q4 | The Stub kind in unit code: is every IC stub inside the unit, or are some called through a link slot? | A2 | Stubs that the unit uses are emitted into the unit; others through a link slot (04 §5) |
| H34-Q5 | README §3.4 asks a Freeze table for a row built after M1 exit. Does a part (the JIT finalizer, M5) need a later freeze point? | M1 exit | The whole row freezes at M1 exit; the finalizers follow it |

## Reopens if

- SP-Unit fails on any tier-1 target (09 §4 names H34 and H35).
- Spike C or SP-Facts fails a correctness or structure gate (04 §5: reopens H29-H34 before M1 exit).
- A unit needs a text relocation that no kind above covers, or the A2 zero-relocation check fails.
- D9 changes (LLVM or another non-Rust backend is allowed).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H34 and 04 §5, with the Spike B emitter evidence: the zero-relocation rule (`emit.rs:19`) and its jit_b test for (b) on AArch64; (a) and x86-64 by design only. The 04 §5 pc-relative AOT form of LinkSlot and IcCell is not copied (it conflicts with H35-Q9) and is H34-Q1. The SP-Unit questions and H07-Q2 stay open (M2-P5-PLAN-D20, plan-v6 D18) | P5-DOCS-SYNC agent (M2-P5) |
| 2026-10-22 | fill | M3-P0 P0-B-X64: x86-64 placement (b) is built (`spikes/spike-b/src/jit_b/x64.rs`), and the two-isolate no-address test covers it (`jit_b::tests::b_code_has_no_ret_and_no_address`, both architectures); the "(b) on x86-64" row, the (b) Rules row, the (a) Rules row and the Spike B evidence row say so. No rule changes its result | P0-B-X64 agent (M3-P0) |
| 2026-10-23 | clarify | Line ref only: the (b) Rules row cites `b_code_has_no_ret_and_no_address` at `jit_b/mod.rs:286-314` (the CX-JIT1 fix added byte checks to the test above it). No rule, check or value changes | M3-P0 integration fixer (CX-JIT1) |
| 2026-10-23 | clarify | The Status line (and the README index row) names x86-64 (b) as the fill row above does: built, the two-isolate test covers it, and its x86-64 runs are in the M3-P0 sandbox and CI (the orchestrator's runs). Text only; no check changes | M3-P0 integration fixer |
| 2026-10-23 | fill | H34-Q1 to H34-Q3 filled from the `spike-b unit` build: LinkSlot and IcCell through the unit-data register; pc-relative branches inside one function only (rodata loads stay allowed, not built); calls out of unit text are indirect through a register (the JIT code range part of Q3 stays for M5). H34-Q4 and H34-Q5 stay open. No existing check changes | M3-P1 SP-UNIT agent |
| 2026-10-23 | clarify | After the H34-Q1 to Q3 fill: the LinkSlot and IcCell rows of the kinds table and the 04 §5 note no longer say that the unit-text form is open; 04 §5 now says unit-data register. The Status line (and the README §5 cell) names the SP-Unit build. No rule or check changes | M3-P1 integration fixer (from the SP-UNIT report) |
