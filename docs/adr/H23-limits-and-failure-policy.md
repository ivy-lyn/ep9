# H23: Limits and failure policy

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). Late: due M0 wk3 |
| Fix by | M0 wk3 |
| Freeze point | M1 exit |
| Origin | v1#21; plan status E (extended 2026-10-20: interrupt polls, inspector, `panic = "abort"`) |
| Crates | `ep9_vm` (stack limits, interrupt polls, termination, out of memory, size limits), `ep9_frontend` (3-layer depth guard), `ep9_heap` (heap limit), `ep9_platform` (stack bounds; signal and SEH mechanism, H28-Q3), `ep9_cli` (exit codes); later `ep9_masm`, `ep9_baseline`, `ep9_opt` (prologue checks) |
| Owner (plan role) | VM lead; lead/front end for the depth guard; JIT/platform lead for prologues and guard pages |
| Validated by | `ep9_vm` stack-limit and termination tests; `ep9_frontend` depth tests (week-3 check "cap+1 gives RangeError"); test262 exec infrastructure rule; Spike B (prologue checks) |
| Plan sources | [02 §6 H23](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [01 §6](../plan/01-front-end.md#6-depth-guard-3-layers-unchanged), [03 §5 Limits](../plan/03-gc.md#5-contract-items-that-exist-from-s0), [07 §2](../plan/07-runtime.md#2-engine-contracts-the-runtime-needs-early) |

## Context

- G6: hostile input gives SyntaxError or RangeError, never an abort. JS recursion must throw a RangeError, so every interpreter and JIT prologue, every recursive Rust builtin and the parser need a stack check [R09].
- D14: termination comes from another thread. [R18]: an inspector session can run JS at any interrupt poll, so polls are also lazy-deopt points (H24, H29).
- Depends on: H14 (frames), H15 (dispatch), H28 (platform mechanism). Depended on by: H24 (debugger polls), H29 (deopt at polls), H32.

## Decision

Limit checks on VM and native stacks: calls, JIT and AOT prologues, recursive builtins, the 3-layer depth guard. Interrupt polls shared with safepoints and tier-up. Embedder interrupt callbacks never run JS; only an inspector session runs JS at a poll. Heap limit + callback. `panic = "abort"` in release; never unwind panics through JIT or AOT frames (02 §6).

**Facts (as built, `dabc38a`).**

| Item | Value today | Where |
|---|---|---|
| VM register stack | 2^20 slots (8 MiB) by default, 1024..=2^28 accepted; overflow gives a catchable RangeError | `crates/ep9_vm/src/isolate.rs:65`, `crates/ep9_vm/src/isolate.rs:167-171` |
| Native stack margin | JS entries and host calls stop at stack low end + clamp(size / 4, 128 KiB, 1 MiB): catchable RangeError | `native_margin` (`crates/ep9_vm/src/isolate.rs:158-161`) |
| Debug threaded dispatch | panics 64 KiB below the native limit (debug builds have no tail calls) | `crates/ep9_vm/src/interp/dsl.rs:290-292` |
| Depth of `function f() { f() }` | ep9 104,855; node 24.22 10,359; d8 15.8 10,360 (RangeError); QuickJS 2026-06-04 1,169 (InternalError) | measured 2026-10-21, Apple M5 Max, release builds |
| CLI isolate thread | 64 MiB native stack (`--stack-mib`) | `crates/ep9_cli/src/args.rs:97`, `crates/ep9_cli/src/run.rs:244-248` |
| Interrupt poll | at `JumpLoop` and function entry, once per budget of 10,000 | `BUDGET` (`crates/ep9_vm/src/interp/mod.rs:63`) |
| Termination | `TerminateHandle` from any thread; the unwinder runs no handler, so no `catch` and no `finally`; the isolate stays usable | `crates/ep9_vm/src/error.rs:228-235`; test `crates/ep9_vm/src/tests/exceptions.rs:421` |
| Heap out of memory | panic "ep9: heap allocation failed" | `crates/ep9_vm/src/runtime.rs:307-314` |
| Heap limit | none: `max_heap_bytes = usize::MAX`; the 64 GiB cage bounds the heap | `crates/ep9_heap/src/heap.rs:92` |
| String length | 2^29 - 24 code units (as V8); concat past it gives RangeError "Invalid string length" | `crates/ep9_vm/src/string.rs:54`, `crates/ep9_vm/src/conv.rs:414` |
| Array length (v0, dense) | 2^26; a larger `length` gives RangeError "Invalid array length" (H18) | `crates/ep9_vm/src/object.rs:145` |
| Front-end depth caps | `AST_CAP` 20,000; `PRESCAN_CAP` 25,000; depth cap + 1 gives RangeError | `crates/ep9_frontend/src/depth.rs:29`, `crates/ep9_frontend/src/depth.rs:37` |
| Release panic strategy | unwind (`[profile.release]` has no `panic` key) | root `Cargo.toml` lines 42-46 |
| Code that catches panics | test262 exec and parse; the front-end worker; the `ep9` exit-70 path | `tools/test262/src/exec.rs:756`, `tools/test262/src/parse.rs:167`, `crates/ep9_frontend/src/worker.rs:96`, `crates/ep9_cli/src/run.rs:249-253` |
| Native stack past the margin (guard page) | Rust std prints "has overflowed its stack" and aborts (exit 134, SIGABRT, macOS arm64); no ep9 signal handler | Rust std; no `sigaction` in `crates/` |

**Fills (2026-10-21).**

| Topic | Rule | Decision |
|---|---|---|
| Panic policy | Release builds unwind until the first JIT or AOT frame exists (Spike B). From then on shipped builds use `panic = "abort"`, and no panic unwinds through a JIT or AOT frame. Before that switch, ep9-test262 exec and parse, the front-end worker and the `ep9` exit-70 path stop relying on `catch_unwind` (for example a child process per shard) | W4-PANIC-ABORT |
| Out of memory | A near-limit callback runs first (the embedder may raise the limit); then a fatal abort with a message. Never a JS exception. Built in months 2-3 | W4-H23 |
| Size limits | String, array and buffer lengths past a limit throw RangeError; the isolate stays usable | W4-H23 |
| Interrupts | Embedder interrupt callbacks never run JS. Only an inspector session runs JS at a poll | W4-H23 |
| Guard pages | A native stack overflow past the margin is fatal (abort with a message). The margin checks exist so that JS never reaches the guard page. The handler, alternate stack and SEH mechanism belong to H28 (H28-Q3) | W4-H23 |

## Rules

| Rule | Check | From |
|---|---|---|
| VM-stack and native-stack overflow give a catchable RangeError; the isolate stays usable | `vm_stack_overflow_is_a_catchable_range_error`, `native_stack_overflow_is_a_catchable_range_error` (every dispatch kind) | M0 wk3 |
| Every nesting form at depth cap + 1 gives RangeError, never an abort | `every_form_is_gated` (`crates/ep9_frontend/src/depth_tests.rs:229`) | M0 wk3 |
| Termination is uncatchable, runs no `finally`, and leaves the isolate usable | `terminate_from_another_thread` | M0 wk3 |
| A panic, timeout or worker crash in ep9-test262 is an infrastructure error (exit 2), never a pass or a fail | decision T262-INFRA; `EP9_TEST262_FAULT` hooks | M0 wk3 |
| JIT and AOT prologues check the stack limit | Spike B; A2 gate | Spike B |
| No panic unwinds through a JIT or AOT frame | `panic = "abort"` in shipped profiles; a CI check of the profile | Spike B |
| The out-of-memory path calls the near-limit callback before it aborts | heap limit test (months 2-3) | M0 months 2-3 |

## Consequences

- **Gains.** Hostile input gives catchable errors (G6). ep9 recurses 10x deeper than node, so recursion that works on node works on ep9. Termination needs no signal.
- **Costs.** Until Spike B, release builds keep unwind tables and landing pads. After the switch, a test262 panic ends its process, so the runner needs crash isolation per shard. An abort on Windows exits with 0xC0000409. No heap limit today: a runaway script uses up to the 64 GiB cage.
- **Follow-ups.** H28-Q3 handler; the near-limit callback and a default heap limit (months 2-3); prologue checks (Spike B); each recursive builtin gets the native check when it lands (from week 4).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Stack overflow, OOM, termination and panic policy constrain early design; JS recursion must throw RangeError; tsrs and oxc abort on deep nesting | [09-critic.md](../research/09-critic.md) |
| [R18] | A worker inspector session's `Runtime.evaluate` ran inside a call-free loop and changed a field type: every interrupt poll is a lazy-deopt point | [18-sound-regions.md](../research/18-sound-regions.md) |
| [P1] | Plan v1 depth probe: 20k levels need about 40 MB of stack (01 §6) | Session-local |
| [M] | Recursion depths above; Rust std guard-page abort (exit 134) | This ADR, measured 2026-10-21 |

## Validation

- Now: the tests above on 4 CI targets; the week-3 depth check; test262 exec reports 0 infrastructure errors (CI run 39518270586).
- Spike B: JIT prologue checks and the abort rule. A2: AOT prologues.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H23-Q1 | When does `panic = "abort"` start? | Spike B | **Filled 2026-10-21: from the first JIT or AOT frame (Spike B)** (W4-PANIC-ABORT) |
| H23-Q2 | What does out of memory do? | M0 months 2-3 | **Filled 2026-10-21: near-limit callback, then fatal abort** (W4-H23) |
| H23-Q3 | Default heap limit | M0 months 2-3 | Decide with the callback; measure node's default first |
| H23-Q4 | VM stack default | M1 | Keep 2^20 slots (H14-Q4) |
| H23-Q5 | Form of the JIT and AOT prologue check | Spike B | One compare with a per-isolate limit, as T0's native check |
| H23-Q6 | Embedder interrupt API | M1 | A callback at a poll; **filled: it never runs JS** (W4-H23) |
| H23-Q7 | Are polls GC safepoints? | S2 | Yes (02 §6 row) |
| H23-Q8 | Termination details: cancel, re-entry | M1 | As V8: uncatchable, skips `finally`, the isolate is reusable (as built) |
| H23-Q9 | Native stack overflow past the margin | M0 wk4 | **Filled 2026-10-21: fatal; mechanism in H28-Q3** (W4-H23) |
| H23-Q10 | oxc layer-1 depth counter | M1 | Upstream patch first (01 §7) |

## Reopens if

- A tier-1 target cannot keep a catchable RangeError for VM or native stack overflow.
- Spike B shows that a JIT frame cannot run with the abort rule (for example an unwinder needs panics).
- D14 changes (11-decisions "Affects": H11, H22, H23).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6, 01 §6, 03 §5, 07 §2 and the as-built limits (M0 wk4 plan, task P0-DOCS 2) | P0-DOCS agent |
| 2026-10-21 | fill | H23-Q1 panic policy (decision W4-PANIC-ABORT, decisions log §11) | P0-DOCS agent |
| 2026-10-21 | fill | H23-Q2 out of memory, size limits, H23-Q6 interrupts, H23-Q9 guard pages (decision W4-H23) | P0-DOCS agent |
| 2026-10-21 | clarify | 16 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]); `MAX_STRING_LENGTH` is now in `string.rs`; the `[profile.release]` lines of the root `Cargo.toml` are 42-46 | P4-DOCS agent |
| 2026-10-21 | clarify | 10 `path:line` references moved to the code after M2-P0 (drift script, [I15]) | M2-P0 integration fixer |
