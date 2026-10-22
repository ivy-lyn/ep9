# H23: Limits and failure policy

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). Late: due M0 wk3 |
| Fix by | M0 wk3 |
| Freeze point | M1 exit |
| Origin | v1#21; plan status E (extended 2026-10-20: interrupt polls, inspector, `panic = "abort"`) |
| Crates | `ep9_vm` (stack limits, interrupt polls, termination, out of memory, size limits), `ep9_frontend` (3-layer depth guard), `ep9_heap` (heap limit), `ep9_platform` (stack bounds; signal and SEH mechanism, H28-Q3), `ep9_cli` (exit codes); later `ep9_masm`, `ep9_baseline`, `ep9_opt` (prologue checks) |
| Owner (plan role) | VM lead; lead/front end for the depth guard; JIT/platform lead for prologues and guard pages |
| Validated by | `ep9_vm` stack-limit and termination tests; `ep9_vm` `tests::isolate_api` (interrupt callbacks, heap limit) and `ep9_heap` `near_limit_tests`; `ep9_frontend` depth tests (week-3 check "cap+1 gives RangeError"); test262 exec infrastructure rule; Spike B (prologue checks) |
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
| VM register stack | 2^20 slots (8 MiB) by default, 1024..=2^28 accepted; overflow gives a catchable RangeError | `crates/ep9_vm/src/isolate.rs:80`, `crates/ep9_vm/src/isolate.rs:254-258` |
| Native stack margin | JS entries and host calls stop at stack low end + clamp(size / 4, 128 KiB, 1 MiB): catchable RangeError | `native_margin` (`crates/ep9_vm/src/isolate.rs:245-248`) |
| Debug threaded dispatch | panics 64 KiB below the native limit (debug builds have no tail calls) | `crates/ep9_vm/src/interp/dsl.rs:307-309` |
| Depth of `function f() { f() }` | ep9 104,855; node 24.22 10,359; d8 15.8 10,360 (RangeError); QuickJS 2026-06-04 1,169 (InternalError) | measured 2026-10-21, Apple M5 Max, release builds |
| CLI isolate thread | 64 MiB native stack (`--stack-mib`) | `crates/ep9_cli/src/args.rs:97`, `crates/ep9_cli/src/run.rs:279-284` |
| Interrupt poll | one budget unit per `JumpLoop` back edge, call and function entry from Rust, per context of the dynamic lookup walk (`poll`, `crates/ep9_vm/src/interp/slow/lookup.rs:117-124`, M2-P2-POLL) and per pass of a builtin loop with neither (`Cx::poll_interrupt`, `crates/ep9_vm/src/abi/index.rs:75-86`); a poll runs when the budget of 10,000 runs out. JIT code reaches the same poll through `RuntimeTable::poll` (`rt_poll`, `crates/ep9_vm/src/native.rs:377`). Every poll runs `slow::interrupt` (`crates/ep9_vm/src/interp/slow/mod.rs:121`): embedder callbacks, then termination | `BUDGET` (`crates/ep9_vm/src/interp/mod.rs:68`); interrupt bits `TERMINATE` = 1, `REQUEST` = 2 (`crates/ep9_vm/src/interp/mod.rs:164`, `crates/ep9_vm/src/interp/mod.rs:167`) |
| Embedder interrupt callback | `InterruptHandle::request(Box<dyn FnOnce() + Send>)` from any thread queues the callback (a `Mutex` queue on the `Runtime`, beside the interrupt word) and sets `REQUEST`; the next poll clears the bit, then runs the queue in request order on the isolate thread. The callback gets no `Cx`: no JS, no JS-heap allocation. It may call `TerminateHandle::terminate` (the same poll stops the script). An idle isolate runs it at its next poll. The handle holds the queue weakly (the `Runtime` owns it): callbacks still queued when the isolate is dropped are dropped with it, unrun, even one that holds a handle; a later request drops its callback at once | `InterruptHandle` (`crates/ep9_vm/src/isolate.rs:197`), `Runtime::run_interrupt_requests` (`crates/ep9_vm/src/runtime.rs:343`); tests `crates/ep9_vm/src/tests/isolate_api.rs:174`, `crates/ep9_vm/src/tests/isolate_api.rs:247` |
| Termination | `TerminateHandle` from any thread; the unwinder runs no handler, so no `catch` and no `finally`; the isolate stays usable | `crates/ep9_vm/src/error.rs:228-235`; test `crates/ep9_vm/src/tests/exceptions.rs:421` |
| Heap out of memory | after one collection, a full sweep of both spaces and the near-limit callback, `Runtime::alloc` panics "ep9: heap limit reached"; release builds abort (`panic = "abort"`). Never a JS exception. Shape and feedback-vector cells take the same heap steps, then panic with their own text ("ep9: shape allocation failed", "ep9: heap allocation failed") | `alloc_failed` (`crates/ep9_vm/src/runtime.rs:414`); `crates/ep9_vm/src/shape.rs:925`, `crates/ep9_vm/src/feedback.rs:164` |
| Heap limit | `HeapConfig::max_heap_bytes` (committed bytes, shape region included); default `usize::MAX`, so the 64 GiB cage (`CAGE_ALIGN`) bounds the heap. `HeapConfig::near_limit: Option<fn(committed, limit) -> usize>`: an allocation that finds no memory calls it once; a higher result becomes the limit (`Heap::max_heap_bytes`) and the allocation retries once | `crates/ep9_heap/src/heap.rs:102`, `crates/ep9_heap/src/heap.rs:78`, `near_limit_raised` (`crates/ep9_heap/src/heap.rs:623`); `crates/ep9_vm/src/shape.rs:159` |
| String length | 2^29 - 24 code units (as V8); concat past it gives RangeError "Invalid string length" | `crates/ep9_vm/src/string.rs:54`, `crates/ep9_vm/src/conv.rs:414` |
| Array length (v0, dense) | 2^26; a larger `length` gives RangeError "Invalid array length" (H18) | `crates/ep9_vm/src/object.rs:161` |
| Front-end depth caps | `AST_CAP` 20,000; `PRESCAN_CAP` 25,000; depth cap + 1 gives RangeError | `crates/ep9_frontend/src/depth.rs:29`, `crates/ep9_frontend/src/depth.rs:37` |
| Release panic strategy | abort: `panic = "abort"` in `[profile.release]` (M0 month 2 P2). `dev-opt` inherits it, so every release and `dev-opt` artifact aborts on a panic: ep9, ep9-test262, ep9-bench, the disasm-check and fp-check builds. The dev profile (`cargo build`, `cargo test`) unwinds; test harnesses, build scripts and proc macros always do. A bin that a test runs through `CARGO_BIN_EXE_*` aborts under `cargo test --release` | root `Cargo.toml` lines 42-52; dep-guards check 13 |
| Code that catches panics (dev profile only: under `panic = "abort"` a panic never unwinds to a catch) | test262 exec and parse, inside their worker processes; the front-end worker; the `ep9` join arm (a fallback: the panic hook exits first) | `tools/test262/src/exec.rs:756`, `tools/test262/src/parse.rs:256`, `crates/ep9_frontend/src/worker.rs:93`, `crates/ep9_cli/src/run.rs:288-291` |
| Panic isolation that needs no `catch_unwind` (as built, M2-P1-ABORT-PRE) | test262 parse and exec run every file in worker processes. A worker that ends (abort, signal, or any panic under `panic = "abort"`) is a `worker crash` infrastructure error, exit 2, and the report line carries the panic text: the worker panic hook writes it to stderr. `ep9`'s panic hook prints `ep9: internal error: <msg> (this is an ep9 bug)` and exits 70 on any thread, so the status stays 70 under `panic = "abort"` (the default hook gives 134) | `in_workers` (`tools/test262/src/exec.rs:1060`), `install_panic_hook` (`tools/test262/src/exec.rs:810`), `parse::worker` (`tools/test262/src/parse.rs:598`), `internal_error` (`crates/ep9_cli/src/run.rs:234`); tests `tools/test262/tests/infra_errors.rs`, `tools/test262/tests/exec.rs`, `crates/ep9_cli/tests/cli.rs` |
| Scope of `panic = "abort"` | One setting per profile for the whole build: Cargo rejects `panic` in a `[profile.*.package.<name>]` table ("`panic` may not be specified in a `package` profile"). A panic that leaves an `extern "C"` function aborts (exit 134), even under `catch_unwind` | measured 2026-10-21 with Rust 1.99.1 (M2-P1-ABORT-PRE) |
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
| No panic unwinds through a JIT or AOT frame | `panic = "abort"` in `[profile.release]` and no other profile setting `panic`: dep-guards check 13 (CI job `dependency guards`). The spike-b workspace sets its own `[profile.release] panic = "abort"` | Spike B (live since M0 month 2 P2) |
| The out-of-memory path calls the near-limit callback before it aborts | `the_near_limit_callback_raises_the_heap_limit_once`, `a_kept_limit_is_the_fatal_heap_limit_panic` (`ep9_vm` `tests::isolate_api`); `a_raised_limit_lets_the_allocation_retry`, `a_kept_limit_gives_out_of_memory_after_one_call`, `alloc_shape_calls_it_too` (`ep9_heap`, both allocation entries) | M0 months 2-3 |
| An embedder interrupt callback runs once, on the isolate thread, at the next poll, and runs no JS | `a_requested_callback_runs_at_the_first_poll` (every dispatch, debug and release), `a_callback_reaches_a_running_endless_loop`, `a_callback_waits_for_a_poll_and_callbacks_keep_their_order` (`ep9_vm` `tests::isolate_api`); no `Cx` in the callback type | M0 month 2 P3 |

## Consequences

- **Gains.** Hostile input gives catchable errors (G6). ep9 recurses 10x deeper than node, so recursion that works on node works on ep9. Termination needs no signal.
- **Costs.** Since the switch (M0 month 2 P2), a panic in a release build ends the process: a test262 panic is a `worker crash` with the panic text from the worker's stderr (crash isolation per file, M2-P1-ABORT-PRE), and `ep9` exits 70 through its panic hook. An abort on Windows exits with 0xC0000409. No default heap limit below the cage (H23-Q3): a runaway script uses up to the 64 GiB cage unless the embedder sets `max_heap_bytes`. An interrupt callback waits for the next poll: up to one budget (10,000 back edges or calls) while JS runs, and until the next run when the isolate is idle.
- **Follow-ups.** H28-Q3 handler; each recursive builtin gets the native check when it lands (from week 4). Done: the near-limit callback and the embedder interrupt callback (M0 month 2 P3), prologue checks (Spike B, H23-Q5).

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
| H23-Q3 | Default heap limit | M0 months 2-3 | **Filled 2026-10-22: no default below the cage.** `HeapConfig::max_heap_bytes` stays `usize::MAX`, so the 64 GiB per-isolate cage bounds the heap; an embedder that wants a limit sets `max_heap_bytes` and `near_limit`. Measured: node 24.22.1 has `heap_size_limit` 4,496,293,888 B (4,288 MiB) on a 128 GiB Apple M5 Max (`v8.getHeapStatistics()`); V8 sizes it from physical memory for its generational old space, which ep9 does not have. A lower default would turn a program that fits in memory into a fatal abort with no JS exception (H23-Q2). Check: `the_default_heap_limit_is_the_cage` (M2-P3-H23-Q3) |
| H23-Q4 | VM stack default | M1 | Keep 2^20 slots (H14-Q4) |
| H23-Q5 | Form of the JIT and AOT prologue check | Spike B | **Filled by Spike B (M2-P2).** The JIT and AOT prologue check is one unsigned compare of sp with the per-isolate limit Cx::native_limit (layout::CX_NATIVE_LIMIT; the value of Runtime::native_limit, stack low + clamp(size/4, 128 KiB, 1 MiB)). AArch64: ldr x16, [x28, #native_limit]; mov x17, sp; cmp x17, x16; b.ls overflow. x86-64: cmp rsp, [r15 + native_limit]; jbe overflow. At or below the limit the prologue calls RuntimeTable::stack_overflow(cx, fp, 0), a catchable RangeError, and returns 0. Entries into native code from T0 and Rust (call_any, call_from_rust) check Runtime::native_stack_ok first. Evidence: spike-b gate check (f), T0<->JIT recursion ends in a catchable RangeError at depth 9007 (816 B/level, mac a64), 9961 (736 B/level, linux a64), 7901 (928 B/level, linux x64); tests::native::deep_t0_native_recursion_is_a_catchable_range_error |
| H23-Q6 | Embedder interrupt API | M1 | A callback at a poll; **filled: it never runs JS** (W4-H23). **Built 2026-10-22 (M0 month 2 P3):** `Isolate::interrupt_handle()` gives a `Send + Sync + Clone` `InterruptHandle`; `request(Box<dyn FnOnce() + Send>)` queues a callback that runs once, on the isolate thread, at the next poll (interrupt bit `REQUEST` = 2). It gets no `Cx`, so it runs no JS and allocates nothing on the JS heap; it may terminate. A callback still queued when the isolate is dropped is dropped with it, never run, and a request after that drops its callback at once (the handle holds the queue weakly, so a callback that holds a handle makes no cycle; P3 review ISO-CYCLE) |
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
| 2026-10-21 | clarify (as built) | Facts: test262 parse runs in worker processes; the worker and `ep9` panic hooks; the two `panic = "abort"` checks (per-profile scope, `extern "C"`). The refs of the CLI-thread and catch rows moved to the new code. No Decision or Rules row changes | M2-P1-ABORT-PRE agent |
| 2026-10-22 | rewrite (as built) | `panic = "abort"` in `[profile.release]` (H23-Q1: the switch comes with Spike B, which merges in the same phase, M0 month 2 P2). Facts: the release panic strategy is abort, and the catch row holds in the dev profile only. Rules: the profile check is live as dep-guards check 13. Costs updated. The kind is rewrite because the panic strategy of the shipped build changes; the Decision text does not | M2-P2-INFRA2 agent |
| 2026-10-22 | rewrite (as built) | Facts: the "Interrupt poll" row names every poll (lookup walk, `Cx::poll_interrupt`, JIT `RuntimeTable::poll`) and the two interrupt bits; new row "Embedder interrupt callback" (`InterruptHandle`, bit `REQUEST` = 2); "Heap out of memory": the near-limit callback runs first and the panic text is now "ep9: heap limit reached" (was "ep9: heap allocation failed"); "Heap limit": `HeapConfig::near_limit` and `Heap::max_heap_bytes`. The kind is rewrite because a message and an API that code depends on change; the Decision text does not. Consequences updated | M2-P3-ISOLATE-API agent |
| 2026-10-22 | fill | H23-Q3 default heap limit: none below the cage (M2-P3-H23-Q3; node 24.22.1 measured). H23-Q5 prologue check (filled by Spike B, text from the P2 hand-off). H23-Q6: the built API form. Rules: the out-of-memory row names its tests; new row for the interrupt callback | M2-P3-ISOLATE-API agent |
| 2026-10-22 | rewrite (as built) | H23-Q6 and the "Embedder interrupt callback" row: what happens to queued callbacks when the isolate is dropped. `InterruptHandle` now holds the queue weakly, so a queued callback that holds a handle no longer keeps the queue alive in a cycle (it leaked before); queued callbacks are dropped with the isolate, and a request after that drops its callback at once (was: "with the last handle"). Tests: a callback holding its own handle, requests racing the isolate drop. Six `path:line` references moved to the new code. The kind is rewrite because the test of a request after the drop changes its result; the Decision text does not | P3 review fix |
