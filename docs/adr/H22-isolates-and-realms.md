# H22: Isolates and realms

| Field | Value |
|---|---|
| Status | Draft (2026-10-21). Late: due M0 wk3 |
| Fix by | M0 wk3 (D14) |
| Freeze point | M1 exit |
| Origin | v1#18; plan status E |
| Crates | `ep9_vm` (`Isolate`, `Runtime`, realm roots, `TerminateHandle`), `ep9_heap` (one `Heap` per isolate), `ep9_platform` (one cage per heap; the process code range, H7); later `ep9_unit` (RW data per isolate), `ep9_runtime` (workers) |
| Owner (plan role) | VM lead; runtime lead for workers (M3) |
| Validated by | `ep9_vm` test `terminate_from_another_thread`; test262 exec (a fresh isolate per run, 4 targets); SP-VA; later the `$262.createRealm` files (M1) and the 109 agent files (M3) |
| Plan sources | [02 §6 H22](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [06 §2.3](../plan/06-object-model-and-spec.md#23-elements-keys-realms-contexts-symbols-h18-unchanged), [07 §2](../plan/07-runtime.md#2-engine-contracts-the-runtime-needs-early), [05 §8](../plan/05-aot.md#8-aot-unit-format-h35), [11 D14](../plan/11-decisions.md#31-d1-d14-from-plan-v1) |

## Context

- D14 (decided 2026-10-20): isolate per thread; only SharedArrayBuffer memory is shared. A shared heap would need a cross-thread safepoint protocol and cross-thread barriers [R09].
- Realms are needed early: `$262.createRealm` is in 284 test262 files (d53ebc8b: built-ins 167, staging 78, language 19, intl402 11, annexB 7, harness 2), and Node needs `vm.createContext` [R08].
- AOT: unit text is shared per process; unit RW data is copied per isolate; realm values come through IntrinsicRef by stable index (05 §8, H13).
- Depends on: D14, H7 (cage, code range), H10 (CSS), H13 (stable indices). Depended on by: H11 (no cross-thread barrier), H20-Q2 (realm of a builtin), H23 (termination), H25 (SAB), H35 (RW copy per isolate).

## Decision

Isolate = one heap, one thread at a time; N realms; workers are isolates; only SAB memory shared. Unit text shared per process; unit RW data per isolate; realm values via IntrinsicRef (02 §6).

```text
 process
   code range (one per process; to be built before Spike B, H7): JIT code + unit text (RX)
   isolate A (thread 1)                      isolate B (thread 2)
     cage A: heap pages (64 GiB VA)            cage B
     VM stack, roots, atoms, units             ...
     realm 0..n (v0: one, in the roots)
   SAB backing stores: off heap, shared (H25)
```

**Facts (as built, `dabc38a`).**

| Fact | Where |
|---|---|
| `Isolate` = `Box<Cx>` + `Box<Runtime>`, not `Send` | `crates/ep9_vm/src/isolate.rs:244-249`; `Heap` holds `PhantomData<*mut ()>` (`crates/ep9_heap/src/heap.rs:190-208`) |
| One heap per isolate, one 64 GiB cage per heap with no code range; one 128 MiB code range per process for JIT code and units of every isolate (`CodeRange::process`, H07-Q6) | `crates/ep9_heap/src/memory.rs:154`; `crates/ep9_platform/src/va.rs:49`, `crates/ep9_platform/src/va.rs:52`, `crates/ep9_platform/src/va.rs:621` |
| The conservative scan finds only cells of its own cage | `Memory::contains` (`crates/ep9_heap/src/memory.rs:262-264`); `find_cell` (`crates/ep9_heap/src/trace.rs:357-358`) |
| Roots and atoms are per isolate (`Shared.roots`) | `crates/ep9_vm/src/runtime.rs:161`, `crates/ep9_vm/src/runtime.rs:179` |
| One realm per isolate in v0 | `Root` (`crates/ep9_vm/src/runtime.rs:51`) |
| `TerminateHandle` (`Arc<AtomicU32>`, the interrupt word `Runtime::terminate`; bit 0 = terminate) works from any thread | `crates/ep9_vm/src/isolate.rs:160-171`; test `crates/ep9_vm/src/tests/exceptions.rs:421` |
| test262 exec makes a fresh isolate per run, one after another on a worker thread | `RunCtx::isolate` (`tools/test262/src/exec.rs:582-589`) |
| `$262` has `global`, `gc` and `evalScript`; `createRealm` is missing | `tools/test262/src/exec.rs:50-54` |
| No shared mutable state between isolates | Statics are read-only tables (`EXIT_STUB`, dispatch `TABLE`), a write-once page-size cache (`OnceLock` in `ep9_platform` `sys`) and a per-thread `MAP_JIT` window counter (`jit`) |

## Rules

| Rule | Check | From |
|---|---|---|
| An isolate runs only on the thread that made it | `Isolate` is not `Send` (compiler) | M0 wk3 |
| Each isolate owns one cage; no cell of one isolate is a root of another | `Heap::new` reserves its own `Cage`; `find_cell` reads only its own chunks | M0 wk2 |
| `TerminateHandle` is the only cross-thread entry into an isolate | `terminate_from_another_thread` (every dispatch kind) | M0 wk3 |
| A `Value` never moves from one isolate to another | Embedder contract; no check yet (H22-Q9) | M0 wk3 |
| JIT code and unit text live in the process code range; unit RW data is copied per isolate | Spike B, SP-Unit; H35 load step 6 | before Spike B |
| AOT code reaches realm values only through IntrinsicRef | H35 zero-relocation check (A2) | A2 |

## Consequences

- **Gains.** No cross-thread barrier or safepoint (H11 keeps one barrier). The CSS range check rejects every cell of another isolate. Dropping an isolate releases its whole cage.
- **Costs.** Each isolate reserves 64 GiB + 128 MiB of address space by default. At most 1,926 default cages fit below 2^47 (17 slots of 4 GiB each), fewer when other mappings take slots. Workers copy data, except SAB memory.
- **Follow-ups.** The process code range (H7, before Spike B). Realm cells and `$262.createRealm` (M1). A smaller cage for workers (H22-Q8, M3).

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R09] | Threading model had no owner: pick isolate-per-thread with only SAB shared (V8) or a shared heap; several realms per heap are needed early | [09-critic.md](../research/09-critic.md) |
| [R08] | `$262` needs createRealm, evalScript, agent; agent tests run on real threads that share SAB memory; 109 agent files | [08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [V8-IG] | V8's standard build puts all isolates in one shared 4 GB pointer cage; isolate groups add more cages. ep9 has no pointer compression (N6), so it needs no shared cage | v8/v8 `src/init/isolate-group.h` lines 50-71 (main, read 2026-10-21) |
| [SM-PEM] | SpiderMonkey reserves one executable region per process: near jumps between JIT pages, and the Win64 exception handler is registered once | mozilla-firefox `js/src/jit/ProcessExecutableMemory.cpp` lines 665-679 (main, read 2026-10-21) |

## Validation

- Now: the `ep9_vm` tests above; test262 exec on 4 targets (102,994 runs per job; each run that compiles gets a fresh isolate; CI run 39518270586).
- M1: the 284 `createRealm` files. M3: the 109 agent files with workers and SAB.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H22-Q1 | One cage per isolate or one per process? | M0 wk3 | **Filled 2026-10-21: one cage per isolate, as built** (W4-H22) |
| H22-Q2 | One process-wide code range for the JIT code of every isolate and for unit text? Today each heap reserves its own, unused | Spike B | **Filled 2026-10-21: yes, built before Spike B** (H7 rewrite, W4-H07-REWRITE) |
| H22-Q3 | What is a realm? | M1 | A heap cell with a stable-index intrinsics table (H13). Functions carry their realm; `Cx.realm` switches on a cross-realm call; IntrinsicRef = `cx.realm.intrinsics[j]` |
| H22-Q4 | Is the read-only snapshot shared or per isolate? | M4 (A1) | Deserialized per isolate in v1 |
| H22-Q5 | May an idle isolate move to another thread (`Send`)? | M1 | No: `Isolate` stays not `Send` |
| H22-Q6 | Several isolates on one thread? | M1 | Allowed, one after another (test262 does this) or nested (a host function of A runs B). Each scan rejects the other cage |
| H22-Q7 | When do realms and `$262.createRealm` come? | M1 | M1 (07 §2: N realms per isolate, built in M1) |
| H22-Q8 | Workers and the SAB backing store | M3 | Workers are isolates on their own threads with a smaller cage; SAB data lives off heap in reserved growable regions, reached through ExternalRef (H25) |
| H22-Q9 | Does `Isolate::call` check that each argument belongs to this isolate? | M1 | Debug builds: the in-cage test on each cell argument |

## Reopens if

- D14 changes to a shared heap (11-decisions "Affects": H11, H22, H23).
- A tier-1 host cannot reserve one cage per isolate for the worker count that M3 needs.
- Spike B needs JIT code next to a cage (for example pc-relative heap access).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-21 | draft | First draft from 02 §6, 05 §8, 06 §2.3, 07 §2 and the as-built isolate (M0 wk4 plan, task P0-DOCS 1) | P0-DOCS agent |
| 2026-10-21 | fill | H22-Q1: one cage per isolate, as built (decision W4-H22, decisions log §11) | P0-DOCS agent |
| 2026-10-21 | fill | H22-Q2: one process code range, built before Spike B (decision W4-H07-REWRITE) | P0-DOCS agent |
| 2026-10-21 | clarify | 11 `path:line` references moved to the code at `479b1cd` (P4-DOCS drift script, [I15]); the roots line of `Shared::new` is now one line | P4-DOCS agent |
| 2026-10-21 | clarify | 7 `path:line` references moved to the code after M2-P0 (drift script); `TerminateHandle` wraps `Arc<AtomicU32>` (the interrupt word, bit 0 = terminate) since P0-VM, not `Arc<AtomicBool>` | M2-P0 integration fixer |
| 2026-10-22 | clarify | 1 `path:line` reference (`Shared`, `runtime.rs:161`) moved after the `InterruptHandle` weak-queue fix (P3 review ISO-CYCLE) | P3 review fix |
| 2026-10-22 | clarify | Facts: `$262` also has `evalScript` since P4-PROMISE (`Cx::evaluate_script`); the `exec.rs` reference now covers the 5 lines of the `PRELUDE` doc and text. No check changes | P4 integration fixer |
| 2026-10-22 | clarify | 3 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
