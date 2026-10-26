# H31: Fuse registry

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the two fuse kinds and the initial list. No registry is built; T0 has one sticky flag that works like the "no-elements prototypes" fuse (As built). The registry is built in M1-M2 |
| Fix by | API frozen M1 exit; built M1-M2 |
| Freeze point | M1 exit for the API (README §2). The registry is built partly after M1 exit (M2), so README §3.4 applies: H31-Q6 |
| Origin | R16-D8; plan status N |
| Crates | `ep9_vm` (the registry, fuse words, dependency lists, lazy deopt trigger); later `ep9_opt` and `ep9_baseline` (`Assume(fuse)`), `ep9_unit` (fuse refs in the link table, H35), `ep9_snapshot` (stable fuse indices, H13) |
| Owner (plan role) | VM lead (the registry lives in `ep9_vm`, 04 §8); AOT senior (SP-Facts reopens H31, 09 §4) |
| Validated by | SP-Facts inside Spike C (M1) reopens H30-H34; fuse tests when the registry is built (M1-M2); inspector tests (M4) for the two debugger fuses |
| Plan sources | [02 §6 H31](../plan/02-architecture.md#6-hard-to-change-decisions-h1-h37), [04 §8](../plan/04-execution.md#8-fuses-h31), [04 §4](../plan/04-execution.md#4-tier-up-dispatch-slots-and-hybrid-re-tier), [06 shapes](../plan/06-object-model-and-spec.md), [H24](H24-debug-and-profiling-hooks.md), [H17](H17-object-layout.md) |

## Context

- JS can patch prototypes, `Symbol.species` and `Promise.prototype.then` at any time. Fast paths and optimized code that assume the default must learn about a change. AOT code cannot be recompiled in place (02 §6).
- Enum objects are mutable at run time, so TS enum folds need a fuse [R16].
- A debugger can rewrite locals and bindings [R18]. H24 keeps PROVEN facts sound through two fuses.
- Depends on: H13 (stable indices: a fuse is named by index in units and snapshots), H14 (dispatch-slot swap), H29 (lazy deopt), H17 (constness bits and replacement watchpoints), H24 (debugger fuses), H23 (interrupt polls).
- Depended on by: H30 (`Assume(fuse)`; fused HINTED facts), H32 (the "global invariant" row), H35 (fuse refs in the RW template), H13 (the fuse list is a stable index list).

## Decision

Copied from the plan row (02 §6): GuardFuse (explicit load + compare) and InvalidatingFuse (dependency → lazy deopt + dispatch-slot swap). Covers protectors, shape constness, TS-derived CHA and enum folds, the InspectorSession fuse (pops on session connect) and the DebuggerBindingWrite fuse (pops before a debugger writes a binding; deopts code that used PROVEN binding facts).

| Kind (04 §8) | In the code | On invalidation | Fast-path cost | Main use |
|---|---|---|---|---|
| GuardFuse | Explicit load + compare of a fuse word (RootIndex or link cell) | Code sees the new value and takes the slow path | One load + branch | AOT-jitless; any code that must not deopt |
| InvalidatingFuse | Nothing; a registered dependency | Lazy deopt of dependent frames (return-address swap) + dispatch-slot swap | Zero | JIT mode, hybrid; jitless where another entry exists |

**Initial list (04 §8).**

| Group | Fuses |
|---|---|
| Builtins | Array species; Array, String, Map and Set iterators; Promise.then lookup; no-elements prototypes; RegExp exec; TypedArray species |
| Shapes | Shape constness and replacement watchpoints (H17) |
| TS facts | TS-derived class-hierarchy and enum facts (A4) |
| Debugger | `InspectorSession` (pops on session connect); `DebuggerBindingWrite` (pops before a debugger writes a binding) (H24) [R18] |

- Fuses are one-way: intact, then popped (04 §8).
- The registry lives in `ep9_vm`. API frozen at M1 exit; built in M1-M2 (04 §8).
- In AOT-jitless mode an InvalidatingFuse lazily deopts and swaps the slot to aot-baseline or interp; it patches no code (04 §4, 05 §10).

**As built (e719ffa).** No fuse registry exists. One T0 flag is a one-way invariant of the initial list: `ShapeState::proto_index` is set once a prototype-mode object gets an array-index own property (since M3-P0 also once a routed ARGUMENTS or MODULE_NAMESPACE object becomes a prototype, as its hooks may answer for an index) and is never cleared (V8's "no elements" protector; `crates/ep9_vm/src/shape.rs:230-235`, read by `shape::proto_index_keys` at `crates/ep9_vm/src/shape.rs:1186-1187`). While it is false, `[[Set]]` of a missing array element skips the prototype walk (`crates/ep9_vm/src/property.rs:415-420`). It is a Rust fast-path check, like a GuardFuse, with no stable index and no dependency list (H31-Q5).

## Rules

| Rule | Check | From |
|---|---|---|
| A fuse is one-way: once popped it never becomes intact again in that isolate | fuse unit tests in `ep9_vm` when the registry is built; M1: Spike C / SP-Facts for the API | M1 |
| A GuardFuse costs one load + compare of a fuse word read through RootIndex or a link cell; it needs no deopt | M1: Spike C / SP-Facts; unit golden tests of the AOT emit (H34 `RootIndex`, `LinkSlot`) | M1; A2 |
| An InvalidatingFuse pop lazily deopts every dependent frame by return-address swap and swaps the dispatch slot; no code is patched | M1: Spike C / SP-Facts (lazy deopt, H29); the "Tier differential + deopt" fuzz row (10-quality §4) | M1; M5 |
| Every fast path that assumes a builtin default (the initial list) checks or depends on its fuse | fast-path differential tests (10-quality §2, "no spec deviation in fast paths") | M2 |
| Shape constness bits and replacement watchpoints register as InvalidatingFuses | H17 rule rows (`docs/adr/H17-object-layout.md:132`); shape tests when constness is built | M1 |
| TS-derived CHA and enum folds exist only under a fuse | hint chaos fuzzing (10-quality §4); hint soundness corpus (10-quality §2) | A4 |
| Session connect pops `InspectorSession`; a debugger write to a context, module or script binding pops `DebuggerBindingWrite` first | inspector protocol tests (H24 Rules, `docs/adr/H24-debug-and-profiling-hooks.md:45-46`) | M4 |
| Fuses have stable indices, so a unit and a snapshot name a fuse by index | H13 stable-index order checks (the fuse list, H13 follow-ups) | M1 |

## Consequences

- **Gains.** Zero-cost invariants in JIT code (InvalidatingFuse). A cheap, deopt-free check in AOT-jitless code (GuardFuse). One mechanism for builtins, shapes, TS facts and the debugger.
- **Costs.** Every fast path must name its fuse. A pop deopts all dependent code at once; a program that patches a builtin early pays it once.
- **Follow-ups.** Build the registry (M1-M2). The fuse list joins the H13 index lists. H32 picks GuardFuse or InvalidatingFuse per mode.

## Evidence

| Tag | Claim used here | Link |
|---|---|---|
| [R14] | V8 has 22 one-way isolate protectors; JSC uses WatchpointSets; SpiderMonkey RealmFuses split guard and invalidating fuses; lazy deopt patches return addresses, not code | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R16] | D8: GuardFuse (load + compare, AOT-cheap) and InvalidatingFuse (lazy deopt + slot swap); TS CHA and enum folds use fuses | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R18] | A node inspector session rewrote strict `let`/`const` locals and retyped a field in a call-free loop; fuses F1-F8 named for sound regions | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| code | `grep -rniwE 'fuses?\|protectors?'` over `crates/*/src` at e719ffa: one match, the `proto_index` doc (`crates/ep9_vm/src/shape.rs:216`) | this ADR |

## Validation

- M1: SP-Facts (reopens H30-H34 on a failed gate, 09 §4). M1-M2: registry tests and fast-path differential tests. M4: inspector tests. A4: hint chaos.

## Open questions

| ID | Question | Needed by | Proposed default |
|---|---|---|---|
| H31-Q1 | The registry API: fuse id type, how code registers a dependency, how a pop finds dependent code and frames | M1 exit (API freeze) | Fuse = stable index (H13); a dependency list per fuse of weak code handles; a pop walks the list, marks code, then swaps return addresses (H29) |
| H31-Q2 | Fuse scope: per isolate or per realm. The builtin fuses guard realm intrinsics (H22) | M1 exit | Per realm for intrinsic fuses, per isolate for the debugger fuses |
| H31-Q3 | The fuse word format for a GuardFuse (where it lives, what "intact" reads as) | M1 exit | One word per fuse in the isolate root table, read through RootIndex (H13) or a unit link cell (H35) |
| H31-Q4 | Do F1-F6 and F8 of [R18] (per-key prototype conditions, HasPrototype, shape stability, field type, array protectors, buffer detach, closed world) join the list? They are for sound regions (A5v) | M7 exit (D17 rule) | Not part of the decision unless the D17 rule picks (d); F5 overlaps the "no-elements prototypes" and "species" entries |
| H31-Q5 | How T0 and Rust builtins (no compiled code) use a fuse. `proto_index` is a private flag today | M1 (registry) | Rust fast paths read the fuse word as a GuardFuse; `proto_index` becomes the "no-elements prototypes" fuse |
| H31-Q6 | README §3.4 asks a Freeze table: the API freezes at M1 exit, the registry is built until M2. Which parts freeze when? | M1 exit | API at M1 exit; the initial list at M2 exit |

## Reopens if

- SP-Facts fails a structure gate (09 §4: H30-H34).
- A builtin invariant cannot be one-way (a fuse would need to re-arm).
- The D17 rule picks (d): fills H31-Q4.

## Change log

| Date | Change kind | Change | By |
|---|---|---|---|
| 2026-10-22 | draft | First draft from 02 §6 row H31 and 04 §8: two fuse kinds, the initial list, the debugger fuses of H24; API, scope and word format are Open questions | P3-ADR-JIT agent (M2-P3) |
| 2026-10-22 | clarify | 1 `path:line` reference moved to the code after the M2-P5 merge and the P5 fixer changes (drift script, [I15]). No check changes | P5 integration fixer |
| 2026-10-23 | clarify | 5 `path:line` references moved to the code after the M3-P0 merge and the M3-P0 code fixes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS). No check changes | M3-P0 integration fixer |
| 2026-10-23 | rewrite (pre-freeze churn, README §3.3; H31 is Draft) | The as-built `proto_index` paragraph: since the M3-P0 code fix CX-VM1 the flag is also set when a routed ARGUMENTS or MODULE_NAMESPACE object becomes a prototype (`shape::may_have_index_keys`), as its hooks may answer for an index key. Test: `layout3` `index_paths_set_walk_and_for_in_reach_the_routed_hooks` | M3-P0 integration fixer |
| 2026-10-23 | clarify | 1 `path:line` reference in `crates/ep9_vm/src/property.rs` moved to the code after the M3-P0 round-2 code fix 02cd320 (X-CODE1: `get_data_from` grew 6 lines; drift script, [I15]). No check changes | M3-P0 round-2 docs fixer |
| 2026-10-23 | clarify | 2 `path:line` references moved to the code after the M3-P1 merge and the M3-P1 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); the 1 ref in the dated Evidence row (the grep at e719ffa) goes back to the line of the commit that row names (M3-P1-REVIEWROW-REFS, X-FD1). No check changes | M3-P1 integration fixer |
| 2026-10-23 | clarify | 2 `path:line` references moved to the code after the M3-P2 merge and the M3-P2 fixer changes (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS); each names the same item at the new line. No check changes | M3-P2 integration fixer (docs) |
| 2026-10-26 | clarify | 1 `path:line` reference moved to the code after the M4-P0 merge (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `docs/adr/H17-object-layout.md` 120 -> 132 (the Representation and const bits row; P0-VM4 added the OBJECT_LAYOUT 4 rows to H17). Text only; no check changes | M4-P0 integration fixer |
| 2026-10-26 | clarify | 2 `path:line` references moved to the code after the M4-P1 merge (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `ep9_vm/src/shape.rs` 224-229 -> 230-235 (the `proto_index` field doc) and 1144-1145 -> 1186-1187 (`proto_index_keys`); VALIDITY f58042c added the validity-cell lines above them. Text only; no check changes (H31 is Draft) | M4-P1 integration fixer |
