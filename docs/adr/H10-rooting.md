# H10: Rooting

| Field | Value |
|---|---|
| Status | Accepted (2026-10-22, M2-P4-ACCEPT; review record complete). Was: Draft (2026-10-20). S0 code built 2026-10-21: root registry and handles in `crates/ep9_heap/src/roots.rs`, CSS in `css.rs` and `heap.rs` (`Heap::collect`), tests in `src/tests/css.rs`. M0 wk4: H10-Q2 filled; `cargo xtask gc-lint` and the debug on-stack check of `Local` built |
| Fix by | M0 wk2, after SP-GC |
| Freeze point | M1 exit |
| Origin | v1#6; plan status E (extended: AOT frames, unit RW data) |
| Crates | `ep9_heap` (CSS, root sets, root-range hook), `ep9_platform` (spill trampolines), `ep9_vm` (`Local`, `Global`, `ScratchRoots`), `ep9_unit` (registers RW ranges) |
| Owner (plan role) | GC lead |
| Validated by | SP-GC (M0 wk2) |
| Plan sources | [02 §6 H10](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [03 §2](../plan/03-gc.md#2-roots), [03 §5](../plan/03-gc.md#5-contract-items-that-exist-from-s0) |

## Context

- rustc cannot emit stack maps. So Rust frames that hold heap pointers cannot be scanned precisely [R05].
- Rooting APIs are the most expensive part of existing Rust engines: Nova about 8k bind/unbind/scope calls, Brimstone about 2.2k Handle uses, Boa refcount rooting 10-18% of mutator time [R05][R07].
- Units add native frames (AOT code) and per-isolate RW data that hold heap references.
- Goals: G2-G4 (zero-cost locals in builtins), G6. Risks: RK3 (CSS misses a pointer), RK4 (a reference hides in a Rust container).

## Decision

Conservative scan of native stacks (Rust, JIT and AOT frames) after a callee-saved spill. Precise elsewhere: `Local<'s,T>`, `Global<T>`, `ScratchRoots`; lint bans `Vec`/`HashMap` of `Local`. Unit RW data visited precisely (registered by the loader).

```
native stack: Rust + JIT + AOT frames ------------> CONSERVATIVE
    naked trampoline spills callee-saved registers first
      aarch64: x19-x28, fp, lr, d8-d15 | SysV x64: rbx, rbp, r12-r15 | Win64: + rsi, rdi, xmm6-xmm15
    -> scan each word -> cage range check -> VO bitmap (small pages) or chunk table (LOS)
       (interior pointers allowed) -> pin for this GC
VM stack: register files (T0; others per D21) ----> PRECISE (every slot is a valid Value; undefined on push)
Global<T> table (embedder, runtime) --------------> PRECISE
ScratchRoots (Rust temporary lists) --------------> PRECISE, scoped
heap objects -------------------------------------> PRECISE (visitor chosen by the header kind byte)
unit RW data (one copy per isolate, H22) ---------> PRECISE, range registered by the ep9_unit loader
    link cells .................................... strong
    IC cells, cached call targets ................. weak, like FeedbackVector slots
```

- AOT frames use the canonical frame (H14) and the same spill trampolines. No stack maps are needed while the heap does not move.
- VM-stack rule: a JIT or AOT frame may keep its register file on the VM stack (D21). Then every slot still holds a valid Value.
- Each isolate scans only its own thread's stack, at a safepoint. Helper threads never scan stacks (D14).

## Rules

| Rule | Check | From |
|---|---|---|
| Spill all callee-saved registers before the scan, per ABI | SP-GC trampolines for a64, SysV and Win64 | M0 wk2 |
| Interior and one-past-end pointers resolve through VO bits in small pages, and through the chunk table (`LargeHead`, `LargeTail`) and the `LargeHeader` for LOS objects (H09) | SP-GC probe cases S1-S3 as unit tests; test `interior_pointer_into_a_large_object_tail` (`crates/ep9_heap/src/tests/css.rs:78`) | M0 wk2 |
| No `Vec`/`HashMap` of `Local` | Lint "no `Vec<Local>`" (M0 wk4); probe case S7. Built as `cargo xtask gc-lint` (`tools/xtask/src/gc_lint.rs`; cases in `tools/xtask/tests/gc_lint/`) and the debug on-stack check in every `Local` read (`crates/ep9_vm/src/abi/local.rs`); see H10-Q2 | M0 wk4 |
| Safepoints are at calls only | Call-only safepoints in every emitter (03 §2) | M0 (T0); M5; A2 |
| Unit RW ranges are registered and removed per isolate | `ep9_heap` root-range hook; GC-stress with AOT frames | A2 |
| Every root source registers a rescan callback (CSS, VM stack, `Global<T>`, `ScratchRoots`, unit RW ranges, embedder roots, the card table of large arrays). S2 runs all of them in every stop-the-world fixpoint; the root-range hook stays callable more than once (H11) | Root-source registry in `ep9_heap`; V3 | M5 |

## Consequences

- **Gains.** Zero-cost locals in Rust builtins. No refcount traffic. AOT frames need no stack maps.
- **Costs.** CSS cannot be proven sound: pointers can hide as derived or one-past-end values, SIMD lanes or packed forms [R05]. Objects found by CSS are pinned for that GC, so S3 skips them.
- **Follow-ups.**
  - If a moving nursery is ever added, JIT and AOT frames switch to precise maps from the H29 deopt metadata (03 §2).
  - H20 builtin ABI uses `Local`, `ScratchRoots` and no `&mut` to a heap object across a JS call.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R05] | CSS probe S1-S7: without a spill, register refs are missed; interior pointers stay live; a ref only in a Rust `Vec` is invisible | [05-gc-design.md](../research/05-gc-design.md) |
| [R09] | Rooting conflict resolved: conservative native frames over a precise heap (JSC, Oilpan, LibJS precedent) | [09-critic.md](../research/09-critic.md) |
| [R07] | Boa refcount rooting costs 10-18% of mutator time | [07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [V8-L] | V8 `Local` derives from `StackAllocated<true>` under `V8_ENABLE_LOCAL_OFF_STACK_CHECK`; its constructor and copy constructor call `VerifyOnStack()` | v8 main: `include/v8-local-handle.h:354-357`, `include/v8-handle-base.h:28-47` (fetched 2026-10-21) |

## Validation

- **SP-GC** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, GC lead): spill trampolines for a64, SysV and Win64; probe cases S1-S7 as unit tests; 0 missed live objects in 10^6 random safepoints per ABI. On fail it reopens H10 and H9.
- Shared command: `bash spikes/sp-gc/gate.sh` (host gate, nonzero exit on failure).
- A2 gate: GC-stress clean with AOT frames on the stack ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H10-Q1 | The dev host is macOS arm64. Win64 runs only on CI runners. x64 SysV runs locally only under Rosetta. Is a Rosetta run enough for the SysV part of the SP-GC gate? | M0 wk2 | **Filled (M0 month 2 P3, M2-P3-OQ-H10-Q1).** No. A Rosetta run is not the gate. The SysV and Win64 parts of the SP-GC gate are the CI jobs `spike gates (linux-x64)` and `spike gates (windows-x64)` (step "SP-GC gate", `.github/workflows/ci.yml:381-383`). The orchestrator's x86-64 sandbox is the local x86 check. Rosetta is hung on the dev host since M0 month 2 P2 (M2-P3-ROSETTA) |
| H10-Q2 | How the lint finds `Vec<Local>` (clippy lint, xtask grep, or type-level `!Send`/lifetime trick) | M0 wk4 | **Filled (M0 wk4, W4-H10-Q2).** Four layers. (1) Types: `Local<'s>` is branded by the host call and `!Send`, so it cannot be `'static` or cross threads (H20). (2) `cargo xtask gc-lint`: a `syn` 2 pass over `crates/ep9_vm/src` and `crates/ep9_builtins/src`, on stable Rust in under 1 s. It flags a stack-only type (`Local`, `Args`, `Rest`, `ArgsBuilder`, `ScratchList`, `PropertyDescriptor`, `JsStr`, `FlatStr`, and any type that holds one by value) inside `Vec`, `VecDeque`, `HashMap`, `HashSet`, `BTreeMap`, `Box`, `Rc`, `Arc`, `RefCell`, `SmallVec` and the other std containers, in a `static` or `thread_local!`, or captured by a closure given to `Box::new`/`Rc::new`/`Arc::new`. `// gc-lint: allow(<reason>)` skips the next item and is listed. (3) Debug builds: every read of a `Local` checks that `&self` lies on the thread's stack (bounds cached per thread), so a `Local` read through `&vec[0]` panics. V8 checks at construction and copy (`V8_ENABLE_LOCAL_OFF_STACK_CHECK`); a Rust `Copy` type has no copy hook, so ep9 checks at the read. (4) GC-stress runs. Not clippy or dylint: a custom lint pass needs a nightly toolchain, and D8 keeps CI on stable. Known gaps: inferred types (`let v: Vec<_> = it.collect()`), a value moved in by a call (`v.push(l)`), a `Local` copied out of a `Vec` before its read, macro bodies that do not parse, and a name used only inside a format string (`Box::new(move \|\| format!("{l:?}"))`). Names match by their last path segment. A `use .. as` alias counts in every file and only adds a meaning, so it never hides a stack-only type; an alias of an unrelated type to a stack-only name needs an allow. Reopen: a GC-stress failure caused by a `Local` in Rust heap memory adds a type-aware dylint job on nightly |

## Reopens if

- SP-GC fails its gate on any ABI (09 §4 names H10 and H9).
- Any GC-stress diff (RK3 early signal) that a CSS miss explains.
- D14 changes to a shared heap. Then stacks of other threads need scanning (11-decisions lists H11, H22, H23; this row is reviewed with them).

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-20 | draft | First draft from 02 §6 and 03 §2 (M0 wk1 list) | ADR agent |
| 2026-10-20 | fill | Rule added from decision H11-termination: every root source registers a rescan callback, dirty LOS cards included; S2 runs all of them in every stop-the-world fixpoint | apply agent |
| 2026-10-21 | clarify | Status: S0 code built in `ep9_heap`. Root sources, weak sources, strong and weak root ranges, global and weak handles, LIFO `ScratchRoots`; CSS spills through `ep9_platform` and scans [sp, stack high) with the end rule (word and word - 1) and interior lookup | heap agent |
| 2026-10-21 | fill | H10-Q2 filled: types (`'s`, `!Send`) + `cargo xtask gc-lint` (syn, stable) + the debug on-stack check in `Local` reads + GC-stress. The "no `Vec`/`HashMap` of `Local`" Rules row names the built checks. Decision W4-H10-Q2 | P2-LINT agent |
| 2026-10-21 | rewrite (kind set 2026-10-22; was clarify) | gc-lint review fixes: an alias in one file never hides a stack-only type in another; a type that holds a `Local` walks its type arguments (`Holder<'s, Local<'s>>`); `Box::new` through an alias; a shadowing binding counts only where it is in scope (closures, `if let`, `while let`). H10-Q2 known gaps add macro bodies that do not parse and format-string names | gc-lint fix agent |
| 2026-10-22 | clarify | Kind cell of the 2026-10-21 "gc-lint review fixes" row set to rewrite (README §3.3, M2-P2-RF-A2-EDIT): the fixes it records (a3ed6b1, "four false negatives") make `cargo xtask gc-lint`, the check of the "no `Vec`/`HashMap` of `Local`" Rules row, flag code it passed before, and the H10-Q2 fill text changed with them | P3-ACCEPT-2 |
| 2026-10-22 | fill | H10-Q1 filled: a Rosetta run is not the gate; the CI linux-x64 and windows-x64 spike-gates jobs are the SysV and Win64 parts of the SP-GC gate, and the x86-64 sandbox is the local x86 check. Every check keeps its result (M2-P3-OQ-H10-Q1) | P3 integration fixer |
| 2026-10-22 | clarify | 2 `path:line` references moved to the code after the M2-P4 merge (drift script, [I15]). No check changes | P4 integration fixer |
| 2026-10-22 | rewrite | Decision diagram and the Rules row "Interior and one-past-end pointers" scoped as built: VO bits exist only for small-page cells; CSS resolves a pointer into a LOS object through the chunk table and its `LargeHeader` (`find_cell`, `crates/ep9_heap/src/trace.rs:357-382`; H09 Rules rows 2 and 3). The Rules row adds the check `interior_pointer_into_a_large_object_tail`. Codex D1 (M2-P4 docs run). Pre-freeze churn (README §3.3); H10 is Draft | P4 review fixer (docs) |
| 2026-10-22 | clarify | Status Draft -> Accepted (M2-P4-ACCEPT). The §3.1 review record is complete: row 1 P3-ACCEPT-2, row 2 Codex (M2-P3-B2-ROW2, the P4 pre-step run and m2p4-docs (D1 fixed in RF1, approve in round 2)), row 3 P4-ACCEPT-3 (and the P4 review checkers for the RF1 rows), row 4 data. The orchestrator did not write the rows it approves. No check changes | orchestrator |
| 2026-10-22 | clarify | 2 `.github/workflows/ci.yml` line references moved down 6 lines after the setup-node step (M2-P4-CI-NODE24; drift script, [I15]). No check changes. Accepted-path change (README §3.2: orchestrator + review record) | orchestrator |
| 2026-10-23 | clarify (proposed; Accepted path) | 2 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). Text only; no check changes. Review record for this amendment (README §3.1): rows 1-3 by agents that did not write it; the orchestrator approves | M3-P1 integration fixer |

## Review record

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer | P3-ACCEPT-2 (did not write the ADR) | Decision = the 02 §6 H10 row. Checked at e719ffa: the CSS spill and scan (SP-GC gate), the debug on-stack check of `Local` reads (`crates/ep9_vm/src/abi/local.rs`), `cargo xtask gc-lint` (`tools/xtask/src/gc_lint.rs`) and its test cases. R1: the 2026-10-21 "gc-lint review fixes" row had the kind clarify, but a3ed6b1 ("four false negatives") made gc-lint flag code it passed before (kind cell set to rewrite). Other kinds: agree (the H11-termination row adds a Rules row and changes no check: fill). Open and due in M0, not filled (reported): H10-Q1 (is a Rosetta run enough for SysV?): Rosetta is hung on this host since M2-P2, and x64 SysV runs on the CI linux-x64 runner and in the orchestrator's x86 sandbox | c68971a |
| 2 Codex | orchestrator (Codex gpt-6.1-sol) | Three runs. No finding on H10 in the first two: M2-P3-B2-ROW2 ("ADR docs" slice of the P3 merge, base 66f08cd) and the P4 pre-step run on every P3 change of H01, H06, H09, H10, H11, H28 and H36, fc368e1 and 077d1fe included (base f8746a4; 3 findings, on H01, H06 and H09). (3) The P4 docs run (slice m2p4-docs: the P4 edits of 12 ADRs, H09 and H10 among them; base 2e6940a = the P4 head 575569f without the slice): needs-attention, 1 finding. D1 (medium), on H09 and H10: the diagram ("VO bitmap (interior pointers allowed)") and the Rules row "Interior and one-past-end pointers resolve through VO bits" do not hold for LOS objects, which have no VO bit and resolve through the chunk table. Fixed by the P4 review fixer (docs) (row 3) | decisions-log M2-P3-B2-ROW2; M2-P4-B2-ROW2; Codex text outside the repo: scratchpad review/m2p3-doc.codex.txt, review/m2p4-b2.codex.txt, review/m2p4-docs.codex.txt |
| 3 Verifier | P4-ACCEPT-3 (wrote none of the rows it checks) | Checked at 0c4c70e. No Codex finding to sort from the first two runs. Rows written by others: the fc368e1 H10-Q1 fill (`.github/workflows/ci.yml:381-383` is the step "SP-GC gate" of the job `spike gates (${{ matrix.name }})`, whose matrix has macos-arm64, linux-x64, linux-arm64 and windows-x64; no CI step uses Rosetta: true; kind fill: agree, the question gets its answer and no check changes); the 077d1fe ref of that fill (the step moved down one line when P3 added a GC-skip comment line: true); the P3-ACCEPT-2 kind cell of the "gc-lint review fixes" row (a3ed6b1 "xtask gc-lint: four false negatives from the P2 review" makes gc-lint flag code it passed: rewrite: agree) and its clarify row. Change-log kinds: agree. Codex D1 of the P4 docs run (row 2), sorted by the P4 Codex verifier (wrote none of the rows): real; first sorted (c), re-sorted (a) by the orchestrator (decision M2-P4-RF1-RESORT: a false Rules row blocks the planned Accept, README §3.1 part 3): the diagram line and the Rules row date from the first draft (ea31dcb), and P4 did not change `ep9_heap`; VO bits are set only for small cells (`crates/ep9_heap/src/heap.rs:669-675`), and `find_cell` resolves a LOS pointer through the chunk table and the `LargeHeader` (`crates/ep9_heap/src/trace.rs:357-382`); `interior_pointer_into_a_large_object_tail` passes. Fixed by a rewrite row, because H10 goes to Accept with this Rules row (pre-freeze churn; H10 is Draft) The kinds of the P4 review-fix rows (2026-10-22, P4 review fixer (docs)) were confirmed by the RF1 docs checker, which wrote none of them (rewrite for a Rules row change, clarify for a kind-cell change). | branch m0m2/p4-accept-3; local run, mac a64, 0c4c70e; P4 Codex verifier: scratchpad m0m2/p4-verify-codex.json; branch m0m2/p4-rf1-docs |
| 4 Data | from the "Validated by" line | SP-GC gate PASS (as in H09). `cargo xtask gc-lint`: 97 files, 0 findings, 1 allow (`crates/ep9_vm/src/abi/local.rs`, a debug test). `cargo test -p xtask --locked`: 67 pass, 7 ignored. `cargo test -p ep9_vm --locked`: 279 pass (debug builds check every `Local` read). Missing: unit RW ranges (A2); the S2 rescan registry (M5). P4 refresh (0c4c70e): gc-lint 97 files, 0 findings, 1 allow; `cargo test -p xtask --locked` 67 pass, 7 ignored. The real CI run 39706792079 on 938d2bc passed 22/22 (tag m2-p3), with the SP-GC gate in the spike-gates job on 4 runners (SysV and Win64 on linux-x64 and windows-x64) | local run, mac a64, e719ffa; logs outside the repo: scratchpad m0m2/accept2/logs/sp-gc.log, gclint.log, xtask.log, vm.log; P4: scratchpad m0m2/accept3/logs/gclint.log, xtask.log |
| 1 Reviewer (amendment 2026-10-22, M2-P4-CI-NODE24 refs) | reviewer agent (did not write the change) | The moved `.github/workflows/ci.yml` refs point at the same text as before the 6-line setup-node step (compared HEAD~1 old lines with the new lines); nothing else changed but the refs and one Change-log row; kind clarify: agree. Approve (README §3.2: the orchestrator wrote the change, so a second agent approves) | M2-P4-CI-NODE24 |
