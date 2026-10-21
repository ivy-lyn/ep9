# H24: Debug and profiling hooks

| Field | Value |
|---|---|
| Status | Draft (2026-10-21): the M0 rules. The inspector is built in M4, the sampler in M6, the AOT parts in A2 |
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
| R1 | Frame pointers in every tier. VM frames link through the caller-fp slot (fp-8, H14). JIT and AOT code keep the native frame pointer (x29 on aarch64, rbp on x86-64) in every function with a frame | T0: the VM frame chain. Rust code: the target default (H24-Q1) |
| R2 | Positions come from the canonical frame's offset slot (fp-1). JIT and AOT code store the bytecode offset there, and publish fp, before every runtime call and poll. The walker never needs a native pc to find the position of a frame below the top | T0: `Runtime::enter` writes the slot at GC points and host calls; the walker uses the return pc of the frame above otherwise (H14) |
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

- M0: the `ep9_vm` and bytecodegen checks above; Spike B checks (c) and (d) on macOS arm64 and Linux arm64.
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
