# H10: Rooting

| Field | Value |
|---|---|
| Status | Draft (2026-10-20). S0 code built 2026-10-21: root registry and handles in `crates/ep9_heap/src/roots.rs`, CSS in `css.rs` and `heap.rs` (`Heap::collect`), tests in `src/tests/css.rs` |
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
    -> scan each word -> cage range check -> VO bitmap (interior pointers allowed) -> pin for this GC
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
| Interior and one-past-end pointers resolve through VO bits | SP-GC probe cases S1-S3 as unit tests | M0 wk2 |
| No `Vec`/`HashMap` of `Local` | Lint "no `Vec<Local>`" (M0 wk4); probe case S7 | M0 wk4 |
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

## Validation

- **SP-GC** ([09 §4](../plan/09-milestones.md#4-spikes), M0 wk2, GC lead): spill trampolines for a64, SysV and Win64; probe cases S1-S7 as unit tests; 0 missed live objects in 10^6 random safepoints per ABI. On fail it reopens H10 and H9.
- Shared command: `bash spikes/sp-gc/gate.sh` (host gate, nonzero exit on failure).
- A2 gate: GC-stress clean with AOT frames on the stack ([03 §7](../plan/03-gc.md#7-gc-test-matrix-per-mode)).

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H10-Q1 | The dev host is macOS arm64. Win64 runs only on CI runners. x64 SysV runs locally only under Rosetta. Is a Rosetta run enough for the SysV part of the SP-GC gate? | M0 wk2 | Rosetta for local runs; the CI runner is the gate |
| H10-Q2 | How the lint finds `Vec<Local>` (clippy lint, xtask grep, or type-level `!Send`/lifetime trick) | M0 wk4 | Decide with the builtin ABI draft (H20) |

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
