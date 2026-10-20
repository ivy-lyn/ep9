# H28: Platform layer

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). SP-JIT-mem passed on 4 targets (2026-10-21); Accept needs the review record (README §3.1) |
| Fix by | M0 wk1-3 |
| Freeze point | M1 exit |
| Origin | v1#26; plan status U (unchanged; unit parts added in A2) |
| Crates | `ep9_platform` (VA reserve, MAP_JIT/W^X under `jit` (module `jit`), stack bounds, signals/SEH, spill trampolines, unwind registration under `unwind` (module `unwind`; `jit` implies it)) |
| Owner (plan role) | JIT/platform lead |
| Validated by | SP-JIT-mem (M0 wk3); SP-GC Win64 trampoline (M0 wk2); SP-VA (M0 wk2) |
| Plan sources | [02 §6 H28](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [08 §2](../plan/08-repo.md#2-crates), [04 §5](../plan/04-execution.md#5-jit-infrastructure-and-the-relocatable-macroassembler), [03 §2](../plan/03-gc.md#2-roots) |

## Context

- D2 (taken 2026-10-20): 4 tier-1 targets from week 1: macOS arm64, Linux x64, Linux arm64, Windows x64 ([decisions log](../decisions-log.md)).
- Windows differs in the places that are hardest to retrofit: callee-saved registers for CSS (rsi, rdi, xmm6-xmm15), SEH unwind registration for JIT code, and VirtualAlloc reserve/commit semantics [R09].
- No research report covered Windows on the engine side; the critic listed it as a missing topic that constrains early design [R09].
- Risks: RK24 (Windows diverges), RK25 (Win64 passes only 4 register arguments).

## Decision

Win64 spill trampoline, SEH unwind registration and VirtualAlloc in `ep9_platform` from M0. Plus unit mapping and unwind registration on 4 targets (A2).

| Concern | macOS arm64 | Linux x64 / arm64 | Windows x64 | When |
|---|---|---|---|---|
| VA reserve (cage + code range, H7) | mmap | mmap | VirtualAlloc | M0 wk2 (SP-VA) |
| CSS spill trampoline (H10) | aarch64: x19-x28, fp, lr, d8-d15 | SysV x64: rbx, rbp, r12-r15; arm64 as macOS | Win64: SysV set + rsi, rdi, xmm6-xmm15 | M0 wk2 (SP-GC) |
| Executable memory (`jit`) | `MAP_JIT`, `pthread_jit_write_protect_np`, `sys_icache_invalidate` | dual mapping or mprotect | VirtualAlloc | M0 wk3 (SP-JIT-mem) |
| Unwind info for generated code | DWARF CFI | DWARF CFI | `RtlAddGrowableFunctionTable` (SEH) | M0 wk3 (SP-JIT-mem) |
| Unit mapping + unwind registration (H35) | blob or `.o` | blob or `.o` | blob or `.o` | A2 |

- Each cell copies 03 §2 and 04 §5. `ep9_platform` owns all of them ([08 §2](../plan/08-repo.md#2-crates)).

**Filled by SP-JIT-mem (2026-10-21; kind: fill).** Data: [2026-10-21-sp-jit-mem.md](../measurements/2026-10-21-sp-jit-mem.md).

| Item | macOS arm64 | Linux x64 / arm64 | Windows x64 |
|---|---|---|---|
| W^X mode (`jit::WxMode::DEFAULT`) | `MapJit`: the code range is one `MAP_JIT` RWX mapping from `Cage::reserve_jit`; `pthread_jit_write_protect_np` per thread | `Protect`: `mprotect` RW <-> RX per write window (H28-Q1) | `Protect`: `VirtualProtect` RW <-> RX |
| Other modes built | `Protect` (plain range, no hardened runtime; cost comparison only) | `DualMap` (memfd, RX view in the code range, RW view outside) | - |
| icache flush after a write | `sys_icache_invalidate` | arm64: `DC CVAU` + `IC IVAU` loop (CTR_EL0); x64: none | `FlushInstructionCache` |
| Unwind registration | `.eh_frame` (absptr FDEs), `__register_frame` once per FDE | `.eh_frame`, `__register_frame` once per section (libgcc) | `RtlAddGrowableFunctionTable`; UNWIND_INFO in the code memory |
| Release of code pages | `madvise(MADV_FREE_REUSABLE)`; pages stay RWX (F2) | `mmap(MAP_FIXED, PROT_NONE)` | `VirtualFree(MEM_DECOMMIT)` |

- Feature split (H27): `jit` = executable memory with W^X. `unwind` = encoders, registration and the native stack walk; the jitless build needs it for AOT units (A2). Unit tests build both modules under `cfg(test)`, so `cargo test --workspace` runs them.
- `CodeMemory::commit` is `unsafe`: the caller owns the pages. ep9_masm will own the code-space allocator (04 §5).

## Rules

| Rule | Check | From |
|---|---|---|
| All platform calls (VA, W^X, unwind, signals/SEH, stack bounds) live in `ep9_platform` | Crate review; `unsafe` only in reviewed crates | M0 |
| CI builds and tests all 4 targets from week 1 | CI matrix | M0 wk1 |
| Generated code can be called, unwound and backtraced on all 4 targets | SP-JIT-mem (`spikes/sp-jit-mem/gate.sh`) | M0 wk3 |
| Every generated frame has registered unwind info before it runs (on Windows an unregistered frame looks like a leaf) | SP-JIT-mem control check; later ep9_masm finalize | M0 wk3 |
| No W^X mode is the default unless it stays coherent under a binary translator (Rosetta) | SP-JIT-mem `icache` check | M0 wk3 |
| Win64 spill trampoline finds every live object | SP-GC on Win64 | M0 wk2 |
| Units register unwind info on load and remove it on unload | A2 gate: exceptions and Error.stack through AOT frames | A2 |

## Consequences

- **Gains.** No late CSS or unwind rework for Windows. The jitless build reuses the same layer without the `jit` parts.
- **Costs.** About 1-3 PM for the Win64 trampoline, SEH and VirtualAlloc (D2: dropping Windows would save it). Windows CI time from week 1.
  - Windows x64 must stay within 10% of Linux x64 on speed gates (est.) from M2 (G10).
- **Follow-ups.**
  - H15: ≤ 4 handler arguments, because Win64 passes only 4 integer arguments in registers.
  - D16 = yes would add iOS arm64 (static `.o` units, no JIT) through this layer (11-decisions "Affects": D16 → D2, H28, H35).
  - `ep9 check` and ep9-typecheck stay unix-only until SP-tsrs-win passes (G10).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Windows matrix: Win64 callee-saved set differs; JIT code needs SEH unwind registration; VirtualAlloc semantics differ; MMTk has no Windows support | [09-critic.md](../research/09-critic.md) |
| [R06] | macOS MAP_JIT + per-thread W^X toggle works for an unsigned Rust binary; a toggle pair costs about 23 ns | [06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R05] | aarch64 callee-saved spill set for the CSS trampoline | [05-gc-design.md](../research/05-gc-design.md) |
| [P1] | Plan v1 probe: a 5th handler argument breaks the tail call on Win64. SP-Dispatch checks it again | Session-local |
| [M-JIT] | SP-JIT-mem: all gate ids pass on 4 targets; toggle costs; facts F1-F7 (MAP_JIT rules, Rosetta and dual mapping, unwind registration forms) | [2026-10-21-sp-jit-mem.md](../measurements/2026-10-21-sp-jit-mem.md) |

## Validation

- **SP-JIT-mem** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk3, JIT/platform lead): one generated function per target; toggle W^X; call it, unwind and take a backtrace (DWARF CFI; `RtlAddGrowableFunctionTable`) on all 4 targets. On fail it reopens H28 and H27.
  - **Result 2026-10-21: PASS** on macOS arm64, Linux x64, Linux arm64, Windows x64 (CI runners) and x86_64-apple-darwin (Rosetta). [M-JIT]
- **SP-GC** (M0 wk2): spill trampolines for a64, SysV and Win64. **SP-VA** (M0 wk2): cage and code range on 4 targets.
- A2 gate: a unit with stub calls loads as blob and `.o` on 4 targets; exceptions and Error.stack through AOT frames.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H28-Q1 | Linux executable memory: dual mapping or mprotect. The plan allows both | SP-JIT-mem (M0 wk3) | **Resolved 2026-10-21 (fill): mprotect is the default.** A pair costs about 3.6 µs on CI VMs, dual mapping about 12 ns. ICs never patch code (04 §5), so windows open only at finalize. Dual mapping runs stale code under Rosetta (F4), needs a memfd, and keeps an RW alias mapped. `DualMap` stays built for later measurement [M-JIT] |
| H28-Q2 | Linux arm64 and Windows x64 cannot run on the dev host (macOS arm64). They run only on CI runners. When do CI runners for all 4 targets exist? | M0 wk1 check ("CI green on 4 targets") | CI owner reports it |
| H28-Q3 | Signal and SEH handling policy (stack overflow, interrupts) belongs to H23 limits; which parts live here | H23 ADR (M0 wk3) | Mechanism here, policy in H23 |

## Reopens if

- SP-JIT-mem fails on any tier-1 target (09 §4 names H28 and H27).
- SP-GC fails on Win64 or SP-VA fails on Windows.
- D2 changes (drop Windows, add Windows arm64) or D16 = yes (iOS) or D5 = WebAssembly in v1 (11-decisions "Affects" for D2, D5 and D16 include H28).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 03 §2, 04 §5 and 08 §2 (M0 wk1 list) | ADR agent |
| 2026-10-21 | fill | SP-JIT-mem passed on 4 targets. Filled the per-target table (W^X mode, icache, unwind registration, release), the `jit`/`unwind` feature split, 2 rules, H28-Q1 (mprotect default on Linux) | jitmem agent |
