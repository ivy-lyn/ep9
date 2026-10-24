# H28: Platform layer

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P4-ACCEPT; review record complete). Was: Draft (2026-10-20). SP-JIT-mem passed on 4 targets (2026-10-21); Accept needs the review record (README §3.1) |
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
| W^X mode (`jit::WxMode::DEFAULT`) | `MapJit`: the code range is one `MAP_JIT` RWX mapping from `CodeRange::process()`; `pthread_jit_write_protect_np` per thread | `Protect`: `mprotect` RW <-> RX per write window (H28-Q1) | `Protect`: `VirtualProtect` RW <-> RX |
| Other modes built | `Protect` (plain range, no hardened runtime; cost comparison only) | `DualMap` (memfd, RX view in the code range, RW view outside) | - |
| icache flush after a write | `sys_icache_invalidate` | arm64: `DC CVAU` + `IC IVAU` loop (CTR_EL0); x64: none | `FlushInstructionCache` |
| Unwind registration | `.eh_frame` (absptr FDEs); `__register_frame` in the mode the self-test picks: per FDE (Apple libunwind) | `.eh_frame`; mode picked by the self-test: whole section with libgcc (a native gnu build), per FDE with LLVM libunwind (zig cc links it statically) | `RtlAddGrowableFunctionTable`; UNWIND_INFO in the code memory |
| Release of code pages | `madvise(MADV_FREE_REUSABLE)`; pages stay RWX (F2) | `mmap(MAP_FIXED, PROT_NONE)` | `VirtualFree(MEM_DECOMMIT)` |

- Feature split (H27): `jit` = executable memory with W^X. `unwind` = encoders, registration and the native stack walk; the jitless build needs it for AOT units (A2). Unit tests build both modules under `cfg(test)`, so `cargo test --workspace` runs them.
- `CodeMemory::commit` is `unsafe`: the caller owns the pages. ep9_masm will own the code-space allocator (04 §5).

**Rewritten by M2-P1 UNWIND-GUARD (2026-10-21; kind: rewrite).** The unix registration mode is chosen at run time, not by target (the old rule was "whole section on linux-gnu"). The unwinder is a link-time fact: a zig cc build for linux-gnu links LLVM libunwind statically with hidden symbols, and then a whole-section registration is ignored (its FDE decoder rejects the CIE), so no frame of generated code unwinds. `dlsym("__unw_add_dynamic_fde")` returns NULL in that build, so it is only a diagnostic.

```text
  first registration (unwind::registration_mode; jit builds use a built-in probe page)
    probe section: CIE | FDE (other code) | FDE (probe)
    whole-section: register, call the probe, walk from its callback
      walk passes the probe and reaches the caller's frames -> WholeSection
      else deregister
    per-FDE: same                                          -> PerFde
    both miss -> error; every later registration fails, native code does not start
```

| Build | Mode chosen | dlsym guess | Evidence |
|---|---|---|---|
| macOS arm64, macOS x64 (Rosetta) | per-FDE (whole-section misses) | per-FDE | `unwind::tests::each_mode_on_this_host` |
| Linux arm64, Rust's gnu link (libgcc_s) | whole-section (per-FDE also passes: libgcc reads from any FDE to the terminator, so whole-section goes first) | whole-section | same test, Docker rust:1.99.1-slim-bookworm |
| Linux arm64, zig cc link (static LLVM libunwind) | per-FDE (whole-section misses) | whole-section (wrong) | same test; before this change `jit::tests::unwind_through_generated_frame` failed in this build |

- A jitless build (`unwind` without `jit`) has no code of its own to run, so it calls `unwind::self_test` on a probe in the first loaded unit's text (SP-Unit, M3-P1; moved by M2-D21-FAIL) before it registers anything. Windows has one form (function tables), so it has no self-test.

## Rules

| Rule | Check | From |
|---|---|---|
| All platform calls (VA, W^X, unwind, signals/SEH, stack bounds) live in `ep9_platform` | Crate review; `unsafe` only in reviewed crates | M0 |
| CI builds and tests all 4 targets from week 1 | CI matrix | M0 wk1 |
| Generated code can be called, unwound and backtraced on all 4 targets | SP-JIT-mem (`spikes/sp-jit-mem/gate.sh`) | M0 wk3 |
| Every generated frame has registered unwind info before it runs (on Windows an unregistered frame looks like a leaf) | SP-JIT-mem control check; later ep9_masm finalize | M0 wk3 |
| unix: the registration mode comes from a run-time self-test (register a probe, call it, walk through it); if no mode passes, native code does not start | `ep9_platform` `unwind::tests` (`selected_registration_mode`, `each_mode_on_this_host`, `missing_fde_fails_both_modes`); x86 sandbox libgcc and zig builds with `EP9_EXPECT_UNWIND_MODE` (the orchestrator's sandbox script, outside the repo; no CI job sets the variable) | M2-P1 |
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
  - `ep9 check` and ep9-typecheck stay unix-only until SP-tsrs-win passes (G10). **SP-tsrs-win passed** on windows-2025 with 0 local patches, in each of 12 CI runs ([data](../measurements/2026-10-20-sp-tsrs-win.md)): by D2, `ep9 check` may support Windows; ep9-typecheck and `ep9c build --hints` wait for the A1h re-check.

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
| H28-Q2 | Linux arm64 and Windows x64 cannot run on the dev host (macOS arm64). They run only on CI runners. When do CI runners for all 4 targets exist? | M0 wk1 check ("CI green on 4 targets") | **Filled (M0 month 2 P3, M2-P3-OQ-H28-Q2).** CI runs on all 4 targets: macos-arm64, linux-x64, linux-arm64 and windows-x64 (the `build-test` matrix, `.github/workflows/ci.yml:102-105`; the `spikes` job uses the same 4). Real CI run 39677340839 (head 5c9034a, tag m2-p2) passed 22/22 checks on the 4 targets |
| H28-Q3 | Signal and SEH handling policy (stack overflow, interrupts) belongs to H23 limits; which parts live here | H23 ADR (M0 wk3) | **Filled 2026-10-21: policy in [H23](H23-limits-and-failure-policy.md)** (a native stack overflow past the margin is fatal; the margin checks give a RangeError first; termination and interrupts use no signal). **Mechanism here:** stack bounds (`current_thread_stack_bounds`, `stack_pointer`), and any signal or SEH handler with its alternate stack. ep9 installs no handler today; Rust std's guard-page handler aborts (H23-Q9; decision W4-H23) |

## Reopens if

- SP-JIT-mem fails on any tier-1 target (09 §4 names H28 and H27).
- SP-GC fails on Win64 or SP-VA fails on Windows.
- D2 changes (drop Windows, add Windows arm64) or D16 = yes (iOS) or D5 = WebAssembly in v1 (11-decisions "Affects" for D2, D5 and D16 include H28).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6, 03 §2, 04 §5 and 08 §2 (M0 wk1 list) | ADR agent |
| 2026-10-21 | fill | SP-JIT-mem passed on 4 targets. Filled the per-target table (W^X mode, icache, unwind registration, release), the `jit`/`unwind` feature split, 2 rules, H28-Q1 (mprotect default on Linux) | jitmem agent |
| 2026-10-21 | fill | H28-Q3 filled: policy in H23, mechanism here (decision W4-H23). Follow-ups: the SP-tsrs-win result (decision W4-SP-TSRS-WIN; [spike review](../measurements/2026-10-21-m0-spike-review.md)) | P0-DOCS agent |
| 2026-10-21 | rewrite | Unwind registration row: the unix mode is chosen at run time by a self-test (whole section, then per FDE), not by target; a failed self-test stops native code. New Rules row. Closes the week-4 open item "WHOLE_SECTION assumes libgcc on linux-gnu" | M2-P1 UNWIND-GUARD agent |
| 2026-10-21 | clarify | W^X mode row: the `MAP_JIT` code range comes from `CodeRange::process()` (`Cage::reserve_jit` is removed) | M2-P1 integration fixer |
| 2026-10-22 | clarify | Rules row "unix: the registration mode comes from a run-time self-test": its x86 sandbox check is the orchestrator's script outside the repo, and no CI job sets `EP9_EXPECT_UNWIND_MODE`; the check cell says so. No check changes | P3-ACCEPT-2 |
| 2026-10-22 | fill | H28-Q2 filled: CI runs on all 4 targets; real CI run 39677340839 (head 5c9034a) passed 22/22 checks. Every check keeps its result (M2-P3-OQ-H28-Q2) | P3 integration fixer |
| 2026-10-22 | clarify | 3 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P4-ACCEPT). The §3.1 review record is complete: row 1 P3-ACCEPT-2, row 2 Codex (M2-P3-B2-ROW2 and the P4 pre-step run (M2-P4-B2-ROW2)), row 3 P4-ACCEPT-3 (and the P4 review checkers for the RF1 rows), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-22 | clarify | 1 `.github/workflows/ci.yml` line reference moved down 6 lines after the setup-node step (M2-P4-CI-NODE24; drift script, [I15]). No check changes. Accepted-path change (README §3.2: orchestrator + review record) | orchestrator |
| 2026-10-22 | clarify | SP-Unit runs in M3-P1, not M2-P4 (M2-D21-FAIL; Codex P5 docs finding D1). Text only; no check changes. Accepted-path change (README §3.2: orchestrator + review record) | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 1 `path:line` reference moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); the 2 refs in dated review-record rows go back to the lines of the commit each row names (M3-P1-REVIEWROW-REFS, X-FD1). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P1 integration fixer |
| 2026-10-23 | clarify (proposed; Accepted path) | 1 `path:line` reference moved to the code after the M3-P2 merge and the M3-P2 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. The ref moved because the M3-P2 docs fixer added 2 lines to the `ci.yml` header comment. Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it, row 4 the ADRLINE output; the orchestrator approves | M3-P2 integration fixer (docs) |
| 2026-10-24 | clarify | Review record for the M3-P2 ADRLINE clarify row: rows 1, 2 and 4; row 3 and the Approval row are pending. No text outside the record changes; no check changes | M3-P2 records writer |
| 2026-10-24 | clarify | Review record for the M3-P2 ADRLINE clarify row: row 3 (the M3-P2 ADRLINE row-3 verifier, checked at df87c4a: every moved ref names the same line text; nothing to sort) and the Approval row (the orchestrator approves, README §3.2). No text outside the record changes; no check changes | M3-P2 ADRLINE row-3 verifier |
| 2026-10-25 | clarify (proposed; Accepted path) | 1 `path:line` reference moved to the code after the M3-P3 merge and the M3-P3 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. The ref moved because GATES-2 changed the `ci.yml` header comment. Line refs moved by M3-P3; no meaning change. Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it, row 4 the ADRLINE output; the orchestrator approves | M3-P3 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H28 row plus the SP-JIT-mem fill. This is the second review of the UNWIND-GUARD rewrite (M2-P1-H28-REWRITE): the run-time self-test is built as written (`crates/ep9_platform/src/unwind/mode.rs`: whole section first, then per FDE, both miss = an error that stops native code; `registration_mode`, `self_test`; `dlsym_guess` is only a diagnostic), and its tests pass here (`selected_registration_mode`, `each_mode_on_this_host`, `missing_fde_fails_both_modes`, `jit::tests::unwind_through_generated_frame`). Kind rewrite: agree (a Decision row changed and a Rules row was added). R1: the self-test Rules row names an x86 sandbox run with `EP9_EXPECT_UNWIND_MODE`; that script is outside the repo and no CI job sets the variable (fixed, clarify). Open and due in M0, not filled (reported): H28-Q2 (CI runners for 4 targets): the real CI runs 22 checks on 4 targets since 2026-10-21 (run 39677340839 on 5c9034a). Other kinds: agree | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | No finding on H28 in either run: M2-P3-B2-ROW2 ("ADR docs" slice of the P3 merge, base 66f08cd) and the P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 included (base f8746a4; 3 findings, on H01, H06 and H09). Its "next steps" asked for an independent check of run 39677340839, which its sandbox could not reach (row 3) | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e. No Codex finding to sort. Rows written by others: the fc368e1 H28-Q2 fill (`.github/workflows/ci.yml:95-98` is the `build-test` matrix with macos-arm64, linux-x64, linux-arm64 and windows-x64; the `spikes` matrix has the same 4, `.github/workflows/ci.yml:309-312`: true). Run 39677340839 on 5c9034a, 22/22: verified from the tag message of m2-p2 (`git cat-file -p m2-p2`) ("Real CI (PR #2): run 39677340839 on 5c9034a, 22/22 pass, mergeStateStatus CLEAN"); the decisions-log SPD-P2 row gives the same run id and head, not the check count. Not checked on GitHub (agents do not run gh). Kind fill: agree. The P3-ACCEPT-2 clarify row: no file under `.github` sets `EP9_EXPECT_UNWIND_MODE`; only `crates/ep9_platform/src/unwind/tests.rs` reads it: true. Change-log kinds: agree | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e |
| 4 Data | from the "Validated by" line | SP-JIT-mem: `bash spikes/sp-jit-mem/gate.sh` PASS on mac a64 (unwind-backtrace 8 checks; W^X toggle pair 29.9 ns with `MAP_JIT`, 564 ns with `mprotect`; Win64 builds and clippy); the x64-darwin Rosetta step SKIP (Rosetta hung). SP-GC PASS (Win64 build, save and SEH codes). `cargo test -p ep9_platform --locked`: 49 pass. Missing: A2 unit mapping. P4 refresh (0c4c70e): `cargo test -p ep9_platform --locked` 49 pass. The Linux and Windows runs (was listed missing): the real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3), with the build-test job and the SP-JIT-mem and SP-GC gates on all 4 targets | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/sp-jit-mem.log, sp-gc.log, platform.log; P4: scratchpad m0m2/accept3/logs/platform.log |
| 1 Reviewer (amendment 2026-10-22, M2-P4-CI-NODE24 refs) | reviewer agent (did not write the change) | The moved `.github/workflows/ci.yml` refs point at the same text as before the 6-line setup-node step (compared HEAD~1 old lines with the new lines); nothing else changed but the refs and one Change-log row; kind clarify: agree. Approve (README §3.2: the orchestrator wrote the change, so a second agent approves) | M2-P4-CI-NODE24 |
| 1 Reviewer (amendment 2026-10-22, M2-P5 SP-Unit schedule) | reviewer agent (did not write the change) | The jitless self-test line now says SP-Unit runs in M3-P1: true (decisions-log M2-D21-FAIL, M2-P4-SPUNIT-M3, M2-P5-PLAN-D25). In d6c6063 nothing else in H28 changed but that line and one Change-log row; the `isolate.rs` part of the commit is a doc comment only (the second paragraph has the same words, the doctest is unchanged). Kind clarify: agree. ADRLINE: 0 FIX, 0 UNFIXABLE. Approve (README §3.2: the orchestrator wrote the change, so a second agent approves) | M2-P5-FIX-D1 |
| 1 Reviewer (M3-P1 amendment: the 2026-10-23 ADRLINE clarify row and its X-FD1 restore) | the M3-P1 Claude reviewer, round 2 (checked 94ed243..80b7986; wrote none of the M3-P1 ADRLINE rows), with a ref check by the M3-P1 round-2 docs fixer (data for this row, not a review of its own text) | Reviewer: no code defect in 94ed243..80b7986, and finding RV2-ADRLINE-REVIEW-ROWS (low): the ADRLINE run 1709c9e moved refs inside 13 dated review-record rows, so those rows cite lines of a later tree than the commit they name (fix: HISTORY pins, or one logged decision). The orchestrator took the pins (M3-P1-REVIEWROW-REFS final, after Codex X-FD1). Kind clarify: agree (text only; no Decision, Rules or check change). Ref check (git show of each line, at the head and at the named commits): 1709c9e moved 3 refs in this ADR: ci.yml 97-100 -> 98-101 in the H28-Q2 row and in review row 3, ci.yml 317-320 -> 318-321 in review row 3. Moved refs that stay: the H28-Q2 row ref ci.yml 98-101 names the `build-test` matrix (macos-arm64 to windows-x64) at the head. Restored: review row 3 (checked at 0c4c70e): ci.yml 98-101 -> 95-98 (the `build-test` matrix) and 318-321 -> 309-312 (the `spikes` matrix); each restored ref names at the named commit the item that the row text names | review output outside the repo: scratchpad m0m3/p1/fixer-result.json (key "review": summary and finding RV2-ADRLINE-REVIEW-ROWS); ref check outside the repo: scratchpad fix2docs/tools/pairs.py and rowrefs.py, drift-show0.txt; row written by the M3-P1 round-2 docs fixer from the reports named here |
| 2 Codex (M3-P1 amendment: the 2026-10-23 ADRLINE clarify row and its X-FD1 restore) | orchestrator (Codex gpt-6.1-sol) | The Codex run on the docs slice of the M3-P1 integration fixer (base 94ed243, head b92754d; it holds the 1709c9e ADRLINE run and its Change-log rows): needs-attention, 5 findings. X-FD1 (medium) is on this amendment: ADRLINE replaced the refs of dated review rows with current-tree lines (its example H25 review row 3, where the new range left out 2 of the 3 stores at bfd5cd8; "other dated review rows received the same substitutions"); it asked to restore the commit-scoped refs. X-FD2 to X-FD5 are on H20, H02 and H14, not on this ADR | Codex text outside the repo: scratchpad m0m3/p1/codex-fix-docs.out; row written by the M3-P1 round-2 docs fixer from the report named here |
| 3 Verifier (M3-P1 amendment: the 2026-10-23 ADRLINE clarify row and its X-FD1 restore) | the M3-P1 round-2 verifier (V3; wrote neither the ADRLINE rows, nor the Codex text, nor the restore) | Checked at b92754d with git show at each named commit. X-FD1: real, (b), made by 1709c9e (and, for some refs, by the earlier ADRLINE runs 375d953, c39f782, d6c8620): 13 review-record rows and the H31 Evidence row cite later-tree lines. For this ADR: review row 3 (checked at 0c4c70e): ci.yml 98-101 -> 95-98 (the `build-test` matrix) and 318-321 -> 309-312 (the `spikes` matrix). Fix spec (decision M3-P1-REVIEWROW-REFS final): restore the true ref at the named commit, add a HISTORY pin for each, and correct the count in each 1709c9e row. The other findings are not on this ADR (X-FD2 to X-FD5, X-B4-1 on H20, H02, H14, H23 Facts; 2 (c) findings on H35, H34 and H13). The restore follows the spec | verifier output (X-FD1..X-FD5, X-B4-1 and 2 (c) findings, sorted), held by the orchestrator; row written by the M3-P1 round-2 docs fixer from the report named here |
| 4 Data (M3-P1 amendment: the 2026-10-23 ADRLINE clarify row and its X-FD1 restore) | M3-P1 round-2 docs fixer (runs only) | ADRLINE (scratchpad p4docs/path_line_drift.py, with 25 new HISTORY pins for the restored refs) at 25fd20a: checked=391 ok=391 fixed=0 unfixable=0 (history=34, pinned=7), exit 0; every ref of this ADR is OK, and each restored ref is OK as history at its named commit. It is run again on the commit that adds these rows | local run, mac a64; logs outside the repo: scratchpad fix2docs/drift3.txt, drift-final.txt; the drift script and its backup path_line_drift.py.pre-p1r2 in scratchpad p4docs |
| Approval (M3-P1 amendment: the 2026-10-23 ADRLINE clarify row and its X-FD1 restore) | the M3-P1 approver: a reviewer agent that wrote none of the M3-P1 rows, none of rows 1-4 and none of the round-1 or round-2 fixes | **Approved** (orchestrator approval under README §3.2, checked by the M3-P1 approver). Checked at 2835f76 (m0m3/p1-fix-docs). Rows 1-4 are present and cite real evidence: row 1 the M3-P1 Claude reviewer's output (scratchpad m0m3/p1/fixer-result.json, key "review", finding RV2-ADRLINE-REVIEW-ROWS: 13 dated review rows moved by 1709c9e, which `git show 1709c9e` confirms); row 2 Codex (m0m3/p1/codex-fix-docs.out: needs-attention, X-FD1 on the H25 review row 3 and the other dated review rows); row 3 the round-2 verifier V3 (output held by the orchestrator; its work files are in scratchpad v3docs); row 4 ADRLINE at 25fd20a (fix2docs/drift3.txt: checked=391 ok=391, history=34, pinned=7). Each moved ref was compared with `git show`: the old ref at 2ba3aa4 (the tree that its last ADRLINE run used; the token is the same at 94ed243) against the new ref at the head, or, for a restored ref, at the commit that its row names. Moved, stays: the H28-Q2 ref ci.yml 97-100 -> 98-101 names the `build-test` matrix (macos-arm64 to windows-x64) at the head, as at 2ba3aa4. Restored, review row 3 (checked at 0c4c70e): ci.yml 97-100 -> 95-98 (the `build-test` matrix) and 317-320 -> 309-312 (the `spikes` matrix): the same text and the same job at 0c4c70e, the numbers that 048a60e wrote. The Change-log counts (1 moved, 2 restored) are true. Note: row 1 says 'Kind clarify: agree', but the reviewer's output gives a kind verdict only for the H23 bound-function clarify; for the ADRLINE rows it reported RV2-ADRLINE-REVIEW-ROWS, said not to reopen the content, and checked the tree before the X-FD1 restore. This approver confirms the kind and checked every restored ref, so README §3.1 row 1 and §3.3 hold. Kind clarify: agree (README §3.3: only line numbers in `path:line` refs change; no Decision, Rules row, check, test or golden file changes). Dated rows: none is false about the past; each restored ref again has the number that the row's author wrote. ADRLINE (scratchpad p4docs/path_line_drift.py, dry run) at 2835f76: checked=391 ok=391 fixed=0 unfixable=0 history=34 pinned=7, exit 0; each restored ref is OK as history at its named commit | local run, mac a64, 2835f76; logs and tools outside the repo: scratchpad m0m3/p1/approver/drift0.txt, drift-show.txt, tools/pairs.py, items.py, items23.py, orig.py |
| 1 Reviewer (M3-P2 amendment: the 2026-10-23 ADRLINE clarify row) | the ADRLINE runners (a mechanical run; they wrote these rows) and, as a reviewer that wrote none of these rows, the M3-P2 records writer | Mechanical: the drift script (scratchpad p4docs/path_line_drift.py) maps each ref through the `git diff -U0` hunks from the commit that wrote it, and moves only the line number; no other text changes. One move, run 1 (084b4c7, ADRLINE after the M3-P2 merge, M3-P2 integration fixer (docs)): the H28-Q2 ref to the `build-test` matrix, `ci.yml` 98-101 -> 100-103, because of the 2 lines that the M3-P2 docs fixer added to the `ci.yml` header comment. The records writer compared the old lines at cbae45d with the new lines at df87c4a: the same 4 matrix lines (macos-arm64, linux-x64, linux-arm64, windows-x64). Kind clarify: agree (README §3.3: only line numbers in `path:line` refs change; no check, test or golden changes) | local read, df87c4a (`git show` of each old and new line); the ADRLINE commits named here |
| 2 Codex (M3-P2 amendment: the 2026-10-23 ADRLINE clarify row) | orchestrator (Codex gpt-6.1-sol) | All runs: `/codex:adversarial-review --model gpt-6.1-sol`, in the background. Run 1 (084b4c7) is in M3-P2 round 2, the docs slice (base 1edad65, head 9052884 = the ADR text of e4b7917): needs-attention, 2 findings, R3-4 on H15 and R3-5 on H22; none on the moved ref or on this row. Open finding: none | Codex texts outside the repo: scratchpad m0m3/p2/codex/r2-docs.out; row written by the M3-P2 records writer from those texts |
| 3 Verifier (M3-P2 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P2 ADRLINE row-3 verifier (wrote no ADRLINE row) | Checked at df87c4a, and again at 00e5107 (the records head; no code change after df87c4a). Run 1 (084b4c7) changes in this ADR only 1 line number (in the H28-Q2 row, live text, not a dated review row) and adds its Change-log row. The ref, by `git show`: `ci.yml` 98-101 at cbae45d and 100-103 at 084b4c7, at df87c4a and at 00e5107 are the same 4 matrix lines of `build-test` (macos-arm64, linux-x64, linux-arm64, windows-x64). The Change-log count (1) is true. Kind clarify: agree (README §3.3: text only; only line numbers in `path:line` refs change; no Decision, Rules row, check, test or golden file changes; the ADRLINE commits change no code). Row 2 against the Codex texts: the M3-P2 round-2 docs slice (base 1edad65, head 9052884) holds this move (its H28 text equals e4b7917); its 2 findings are R3-4 on H15 and R3-5 on H22, none on this ADR: true. Findings to sort: none, so there is nothing to sort. Rows 1, 2 and 4: true. Open (a)/(b): none | local read, df87c4a and 00e5107 (`git show` of each old and new line; `git diff -U0` of each ADRLINE commit); tools and logs outside the repo: scratchpad m0m3/p2/row3v/refcheck.py, refcheck.txt, drift-pre.txt, drift-post.txt; Codex texts: scratchpad m0m3/p2/codex/r2-docs.out, r3-fix3-docs.out |
| 4 Data (M3-P2 amendment: the 2026-10-23 ADRLINE clarify row) | the ADRLINE runners (runs only), and the M3-P2 records writer (the run at df87c4a) | ADRLINE dry runs after each run: at 084b4c7 checked=394 ok=394 fixed=0 unfixable=0; at e4b7917 checked=397 ok=397 fixed=0 unfixable=0; at 94e4d52 checked=402 ok=402 fixed=0 unfixable=0. At df87c4a (before this records commit): checked=407 ok=407 fixed=0 unfixable=0, history=38, pinned=6 | local runs, mac a64; logs outside the repo: scratchpad m0m3/p2/docsfix/drift-final.txt, m0m3/p2/fix2docs/drift-final.txt, the fix round 3 workflow output (key result.merge), m0m3/p2/records/drift-pre.txt |
| Approval (M3-P2 amendment: the 2026-10-23 ADRLINE clarify row) | orchestrator (README §3.2; it wrote none of these rows and none of rows 1-4); row written by the M3-P2 ADRLINE row-3 verifier (the M3-P2 records writer wrote its Pending form) | **Approved.** Orchestrator approves (README §3.2, clarify on an Accepted ADR, review record complete; the ADRLINE rows were written by role and fixer agents, not the orchestrator). 2026-10-24. Rows 1-4 are present, and row 3 finds no defect | orchestrator decision (M3-P2 decisions: the ADRLINE clarify rows on H10, H11, H23, H24, H25 and H28 get row 3 by a non-author agent, then the orchestrator approves); rows 1-4 above |
