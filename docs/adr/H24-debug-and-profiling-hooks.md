# H24: Debug and profiling hooks

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P5-ACCEPT; review record complete). Was: Draft (2026-10-21): the M0 rules. The inspector is built in M4, the sampler in M6, the AOT parts in A2 |
| Fix by | M0 rules; AOT parts A2 |
| Freeze point | Freeze table (README §3.4): rules at M1 exit; AOT parts at the A2 gate |
| Origin | v1#22; plan status E |
| Crates | `ep9_vm` (frames, positions, the unwinder, Error.stack), `ep9_frontend` (BytecodeGen positions, no binding folds), `ep9_bytecode` (position tables), `ep9_unit` (A2 maps), `ep9_platform` (perf map, jitdump) |
| Owner (plan role) | Runtime lead (inspector, profiler); VM lead (frames, positions); JIT/platform lead (JIT frames, perf map) |
| Validated by | `ep9_vm` exception and frame tests; bytecodegen goldens; SPIKEB(structural) checks (c) and (d); inspector tests (M4) |
| Plan sources | [02 §6 H24](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [07 §2](../plan/07-runtime.md#2-engine-contracts-the-runtime-needs-early), [H14](H14-frames-and-dispatch.md), [H31 row](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37) |

## Context

- The inspector (CDP Debugger and Runtime) and the sampling profiler must work in every mode (JIT, AOT-jitless, hybrid) with no redesign (G5, 07 §2).
- A debugger can change a binding that compiled code treats as a fact. In node, a session rewrote strict `let` and `const` locals (B28, B28c) and retyped a field inside a call-free loop [R18]. So PROVEN facts stay sound only if every such write goes through a fuse or a T0 frame.
- Depends on: H14 (canonical frame, pc→code registry), H31 (InspectorSession and DebuggerBindingWrite fuses), H29 (deopt), H35 (unit maps). Depended on by: H30 (fact provenance), H32 (guard policy).

## Decision

Copied from the plan row: lazy position tables, kept ScopeInfo, a debug bytecode copy, frame pointers everywhere, perf jitdump. Session connect pops the InspectorSession fuse (H31): deopt all, AOT frames lazily. While a session is connected, T2 and AOT code treat every interrupt poll as a heap effect point. A debugger write to a local or parameter succeeds only on a T0 frame; on a JIT or AOT frame it returns a protocol error. Breakpoints and `debugger` statements deopt to T0 before the pause. A write to a context, module or script binding first pops the DebuggerBindingWrite fuse (H31). BytecodeGen never folds bindings. Units carry pc→bytecode and position maps and a perf map (02 §6).

**The M0 rules.**

| # | Rule | As built (55c15d0) |
|---|---|---|
| R1 | Frame pointers in every tier. VM frames link through the caller-fp slot (fp-8, H14). JIT and AOT code keep the native frame pointer (x29 on aarch64, rbp on x86-64) in every function with a frame | T0: the VM frame chain. Rust code: the target default (H24-Q1). Spike B JIT (M0 month 2): every function keeps the frame record (x29/x30 on AArch64, rbp on x86-64; `spikes/spike-b/src/jit/a64.rs:123-124`) |
| R2 | Positions come from the canonical frame's offset slot (fp-1). JIT and AOT code store the bytecode offset there, and publish fp, before every runtime call and poll. The walker never needs a native pc to find the position of a frame below the top | T0: `Runtime::enter` writes the slot at GC points and host calls; the walker uses the return pc of the frame above otherwise (H14). Spike B JIT (M0 month 2): each table call passes (cx, fp, bytecode offset), and the table function publishes fp and stores the offset through `Runtime::enter` before any of its work (`native::enter`, `crates/ep9_vm/src/native.rs:404`; M2-P1-NE-OFFSET). The JIT code itself writes no VM stack field |
| R3 | Local writes only on T0 frames. A write to a local or parameter of a JIT or AOT frame is a protocol error; context, module and script bindings pop DebuggerBindingWrite first | No debugger yet (M4) |
| R4 | BytecodeGen never folds a binding: each read of a `let`, `const`, `var` or parameter loads the binding | `const j = 7; return k + j` reads `j` with `Ldar r0` and `k` with `LdaGlobal` (checked with `ep9 --print-bytecode`) |
| R5 | Position tables: UTF-8 offsets per instruction, with a statement bit for breakpoints and stepping. Line and UTF-16 column are computed only when asked | `Function::positions`; `LoadedUnit::line_column` |
| R6 | ScopeInfo is kept: the unit's scope descriptors live as long as the isolate | `Runtime::units` keeps every loaded unit |

## Rules

| Rule | Check | From |
|---|---|---|
| R1: VM frames form one chain that the walker follows from the top frame | `frame::tests::*`; `tests::exceptions::*` (Error.stack walks it) | M0 wk3 |
| R1: JIT code keeps a frame-pointer chain that a native backtrace can follow | SPIKEB(structural) check (d): a native backtrace from a runtime fn passes through JIT frames | Spike B (M0 month 2) |
| R2: Error.stack positions of T0 frames come from the return pc or the offset slot | `tests::exceptions::*` (file:line:col, UTF-16 column) | M0 wk3 |
| R2: a JIT frame's position comes from its offset slot | SPIKEB(structural) check (c): Error.stack shows the JIT function at the right line | Spike B (M0 month 2) |
| R4: no binding folds | `bytecodegen::tests::golden_disassembly` (for example `var-closure`, `assign-const`) | M0 wk3 |
| R5: every throwing instruction has a position; statement positions exist | `bytecodegen::tests::throwing_instructions_have_positions`; `verifier::position_rules` | M0 wk3 |
| R3: local writes on JIT or AOT frames fail; binding writes pop DebuggerBindingWrite | inspector protocol tests | M4 |
| Session connect deopts all frames (AOT frames lazily); breakpoints and `debugger` deopt to T0 | inspector and H31 fuse tests | M4 |
| A connected session makes every T2 and AOT poll a heap effect point | T2 and AOT tests with a session | M7 (T2), A3 |
| Units carry pc→bytecode, position maps and a perf map | A2 unit tests | A2 |

## Consequences

- **Gains.** Error.stack, the unwinder, breakpoints and later deopt read one slot per frame, in every tier. A sampler can walk VM frames from a suspended thread. PROVEN facts need no guard against debugger writes.
- **Costs.** JIT and AOT code pay one store per runtime call and poll (R2), and keep a frame pointer. A debugger cannot change a local of a JIT or AOT frame without a deopt.
- **Follow-ups.** H31 fuses (M1). The inspector (M4). The sampler (M6). Perf map and jitdump (H24-Q2).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | A sampler walks interpreter, JIT and Rust frames from a signal or a suspended thread: it needs frame pointers everywhere and findable interpreter frames | [../research/09-critic.md](../research/09-critic.md) |
| [R18] | A node inspector session rewrote strict `let` and `const` locals (B28, B28c) and retyped a field in a call-free loop | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| target spec | `rustc +nightly-2026-10-04 -Z unstable-options --print target-spec-json`: frame-pointer `non-leaf` on aarch64-apple-darwin and aarch64-unknown-linux-gnu, `always` on x86_64-apple-darwin, no key (may omit) on x86_64-unknown-linux-gnu and x86_64-pc-windows-msvc (run 2026-10-21) | H24-Q1 |

## Validation

- M0: the `ep9_vm` and bytecodegen checks above; Spike B checks (c) and (d) in the CI step "Spike B structural gate" on all 4 runners (macOS arm64, Linux x64, Linux arm64, Windows x64; no check is skipped on Windows, `.github/workflows/ci.yml:390-396`).
- M4: CDP Debugger tests per mode. M6: the sampler walks mixed T0, JIT and Rust stacks. A2: unit maps.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H24-Q1 | Rust code on x86-64 Linux and Windows may omit frame pointers (target spec above). Force them (`-C force-frame-pointers=yes`)? The T0 handlers are tail-called (H15), so a forced frame adds instructions per dispatch | M6 (sampler) | Measure BENCH-SET and `xtask disasm-check` with the flag at M1; force it if the cost is ≤ 1% |
| H24-Q2 | Perf map and jitdump format and switch | M6 (JIT), A2 (units) | Linux perf map (`/tmp/perf-<pid>.map`) and jitdump behind a runtime flag, written from the code registry (H14) |
| H24-Q3 | Where does the debug bytecode copy live (breakpoint patching)? | M4 | A per-function copy made at the first breakpoint; the dispatch slot points at it while a session is connected |

## Reopens if

- Spike B checks (c) or (d) fail, or the sampler cannot walk JIT or AOT frames (M6).
- An inspector feature needs a write to a JIT or AOT frame (R3 changes).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft (M0 rules R1-R6) from 02 §6 row H24 and 07 §2, with the facts as built at 55c15d0 and the frame-pointer defaults of the 5 targets | P0-DOCS agent (M2-P0) |
| 2026-10-22 | clarify | Validation: Spike B checks (c) and (d) run on all 4 CI runners since M0 month 2 P2 (the CI spikes job), not on macOS arm64 and Linux arm64 only. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | M0 rules R1 and R2, As built column: the Spike B facts (the JIT frame record; the table function stores the offset and publishes fp through `Runtime::enter`). The rules do not change. No check changes | P4-ACCEPT-3 |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | clarify | 1 `.github/workflows/ci.yml` line reference moved down 6 lines after the setup-node step (M2-P4-CI-NODE24; drift script, [I15]). No check changes. | orchestrator |
| 2026-10-22 | clarify | Review record row 2 names the P5 Codex run (slice accept4): 2 findings, A1 on H23 and A2 on H04, both fixed (clarify); no finding on this ADR. Text only; no check changes | orchestrator |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P5-ACCEPT). The §3.1 review record is complete: row 1 P4-ACCEPT-3, row 2 Codex (m2p4-docs run; P5 accept4 run), row 3 P5-ACCEPT-4 (with the P5 round-2 notes), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 2 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P0 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P4-ACCEPT-3 (did not write the ADR) | Decision = the 02 §6 H24 row, word for word. Checked at 0c4c70e: the tests the Rules name exist and pass (`frame::tests`, `tests::exceptions`, `golden_disassembly` with the `var-closure` and `assign-const` goldens, `throwing_instructions_have_positions`, `position_rules`); SPIKEB(structural) checks (c) (Error.stack through a JIT frame at the right line) and (d) (a native backtrace through JIT frames) are in `spikes/spike-b/src/structural.rs`; the Spike B prologues keep the frame record; `Runtime::units` keeps every unit (R6); the target-spec evidence re-run with `rustc +nightly-2026-10-04`: non-leaf (aarch64 darwin and linux), always (x86_64 darwin), no key (x86_64 linux and windows): true. R1: Validation named 2 runners for checks (c) and (d); the CI step runs them on 4 (fixed, clarify). R2: the As built column of R1 and R2 was from 55c15d0, before Spike B; R2's "JIT code stores the offset before every runtime call" holds through the callee: the table function stores it in `native::enter` before its body runs (fixed, clarify; the rule text stays). Open questions due in M0: none (Q1 M6, Q2 M6 and A2, Q3 M4). Change-log kinds: agree | branch m0m2/p4-accept-3 |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | The P4 docs run (slice m2p4-docs: the P4 edits of 12 ADRs, among them every B3 record and review fix in H04, H23, H24, H25 and H26; base 2e6940a = the P4 head 575569f without the slice): needs-attention, 1 finding, D1 on H09 and H10; no finding on H24. The ADRLINE ref moves of 375d953 came after the slice and were not in it, nor was the c39f782 ci.yml ref move (Validation, after the setup-node step). P5 run (slice accept4: the P5-ACCEPT-4 commits dffd611 and e4767c1, base bfd5cd8; Codex gpt-6.1-sol): needs-attention, 2 findings: A1 on H23 (host-function calls take no budget unit; fixed 6684988, clarify) and A2 on H04 (the spike-native match-loop fallback; fixed c4ca7de, clarify). No finding on this ADR. | Codex text outside the repo: scratchpad review/m2p4-docs.codex.txt (file list review/m2p4-docs.list); Codex text outside the repo: scratchpad m0m2/p5/codex-accept4.out |
| 3 Verifier | P5-ACCEPT-4 (wrote none of the rows it checks) | Checked at bfd5cd8. Rows 1 and 2 stay as written; the row 2 note now also names c39f782 (no H24 finding in the m2p4-docs Codex text: true). Rows written by others after the Codex slice: the 375d953 clarify row (2 refs): `native::enter` at `crates/ep9_vm/src/native.rs:404` (R2, As built) is the `unsafe fn enter` that every table function calls first: true; the ci.yml ref moved 382-388 -> 384-390 with the 2 new header lines. The c39f782 clarify row: 384-390 -> `.github/workflows/ci.yml:390-396` after the 6-line setup-node step (9937256); those lines are the comment and step "Spike B structural gate" of the `spikes` job, whose matrix has the 4 runners (macos-15, ubuntu-24.04, ubuntu-24.04-arm, windows-2025): true. Both clarify: agree (no check changes). Open questions due in M0: none (Q1 M6, Q2 M6 and A2, Q3 M4; the table is unchanged since 90db5ee). Part 4 again at bfd5cd8: `frame::tests` 3 and `tests::exceptions` 9 pass; `golden_disassembly` and `throwing_instructions_have_positions` pass; `cargo test -p ep9_bytecode --locked` 75 pass, 2 ignored, with `position_rules`. SPIKEB(structural) checks (c) and (d) not re-run here (the CI step on 4 runners; P5-GATES runs SPIKEB(structural) on the P5 tree). Change-log kinds: agree | branch m0m2/p5-accept-4; local run, mac a64, bfd5cd8; logs outside the repo: scratchpad m0m2/p5/accept4/logs/vm.log, frontend.log, bytecode.log |
| 4 Data | from the "Validated by" line | `cargo test -p ep9_vm --locked --lib` (filters `tests::exceptions`, `frame::tests`, ...): 27 pass, with the 3 `frame::tests` and 9 `tests::exceptions`; `golden_disassembly` and `throwing_instructions_have_positions` pass; `cargo test -p ep9_bytecode --locked` 75 pass, with `position_rules`. SPIKEB(structural) with (c) and (d): the CI step on 4 runners in the real CI run 39706792079 on 938d2bc, 22/22 (tag m2-p3); not re-run here. Missing: inspector tests (M4), the sampler (M6), A2 unit maps | local run, mac a64, 0c4c70e; logs outside the repo: scratchpad m0m2/accept3/logs/vm.log, frontend.log, bytecode.log |
| Approval (M3-P0 amendment: the 2026-10-23 ADRLINE clarify row) | M3-P0 approver: a second reviewer agent (README §3.2); it wrote none of the M3-P0 rows | **Not approved**: the review record is not complete. Checked at 70d79e5 (m0m3/p0-fix-docs). Content: true. The 2 refs are the only text change in this ADR besides the Change-log row (`git diff --word-diff 6b652e1..70d79e5`), and each new ref names the item its text names: `native::enter`, the `unsafe fn enter` of every table function. ADRLINE: 0 FIX, 0 UNFIXABLE. Kind clarify: agree. Missing: rows 1-3, which the Change-log row names (README §3.1); none is written. No content change is needed. Approve again once the rows are written, or once the orchestrator logs that a ref-only move needs only one reviewer row (as the 2026-10-22 H28 approvals had) | local read, 70d79e5; scratchpad approve-p0/refpairs.py; ADRLINE (scratchpad p4docs/path_line_drift.py) |
