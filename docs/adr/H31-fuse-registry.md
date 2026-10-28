# H31: Fuse registry

| Field | Value |
|---|---|
| Status | Draft (2026-10-22): the plan row, the two fuse kinds and the initial list. No registry is built; T0 has one sticky flag that works like the "no-elements prototypes" fuse (As built). The registry is built in M1-M2 |
| Fix by | API frozen M1 exit; built M1-M2 |
| Freeze point | Parts (the Freeze table, README §3.4; H31-Q6): the API at M1 exit; the static fuse list and the dynamic descriptor kinds as append-only lists; the A5v fuses at the end of A5, only if built |
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

**As built (e719ffa).** No fuse registry exists. One T0 flag is a one-way invariant of the initial list: `ShapeState::proto_index` is set once a prototype-mode object gets an array-index own property (since M3-P0 also once a routed ARGUMENTS or MODULE_NAMESPACE object becomes a prototype, as its hooks may answer for an index) and is never cleared (V8's "no elements" protector; `crates/ep9_vm/src/shape.rs:273-278`, read by `shape::proto_index_keys` at `crates/ep9_vm/src/shape.rs:1230-1231`). While it is false, `[[Set]]` of a missing array element skips the prototype walk (`crates/ep9_vm/src/property.rs:469-474`). It is a Rust fast-path check, like a GuardFuse, with no stable index and no dependency list (H31-Q5).

**Fast paths that assume a fuse (M4-P3; MODES-CHECK-w14 MC-w14-18).** No registry exists yet, so each fast path below tests its invariant itself (or, for 2 and 3, relies on a v0 limit); the registry (month 5) makes each a fuse of the list above or an append to it. Named by KEYED and REGEXP1 in their M4-P3 reports and checked at the M4-P3 merge head:

| # | Fast path | Invariant it assumes | Fuse | Test today |
|---|---|---|---|---|
| 1 | KEYED: the keyless int-index paths stop a hole or missing-index read at the receiver, and `[[Set]]` adds a missing array element without the prototype walk (`crates/ep9_vm/src/property.rs:474`, `crates/ep9_vm/src/property.rs:1741`) | no prototype has an index key | "no-elements prototypes" (`proto_index`) | `shape::proto_index_keys`, sticky (`crates/ep9_vm/src/shape.rs:1230`) |
| 2 | KEYED: the keyed fast hit on a present Array element (`present_element`, `crates/ep9_vm/src/keyed_ic.rs:87`) | every array element is a data property {w, e, c} | none in the list yet (with the month-5 element kinds, H18) | v0 limit: `Object.freeze`, `Object.seal` and a non-{w, e, c} `defineProperty` of an element throw "not implemented" (`crates/ep9_vm/src/property.rs:1320-1321`); the month-5 element kinds must add a check to `present_element` |
| 3 | KEYED: the keyless get and `in` paths reach `own_index` for every object | only the routed kinds (ARGUMENTS, MODULE_NAMESPACE) answer an index through hooks | none in the list yet | `exotic::is_routed` (`crates/ep9_vm/src/exotic.rs:238`); PROXY (M4-P4 PROXY1) must join it or get its own case |
| 4 | REGEXP1: RegExpExec keeps the match as a `MatchRecord` and makes no result array; `[Symbol.split]` with `%RegExp%` as the species constructor searches with one non-sticky program (`crates/ep9_builtins/src/regexp.rs:24-30`) | `exec` (looked up on the RegExp; for split, the own `exec` of `%RegExp.prototype%`) is the builtin `exec` of the current realm | "RegExp exec" | `Cx::is_builtin` on every call (`crates/ep9_builtins/src/regexp/exec.rs:360-361`, `crates/ep9_builtins/src/regexp/symbols.rs:618-627`); no flags fuse |

**PROXY2 caller list (dated 2026-10-27, M4-P4 integration fixer B; R-VM-P4-3, Codex PROXY1 P-3; the month-5 PROXY2 row reads it with row 3 above).** PROXY1 (M4-P4) made PROXY and PROXY_CALLABLE routed kinds (`exotic::is_routed`, kinds 25-28, `crates/ep9_vm/src/exotic.rs:238`), so row 3 holds for them. Until PROXY2 brings the `Proxy` constructor no JS can make a proxy; these callers keep an ordinary (infallible) form or a proxy-blind read, and PROXY2 decides each, at the lines of the M4-P4 head:

| Caller | Form | Why, and what PROXY2 does |
|---|---|---|
| `crates/ep9_vm/src/interp/slow/runtime.rs:558` | `own_property_keys` | routed: CopyDataProperties `{...p}` takes any source; switch to `try_own_property_keys` |
| `crates/ep9_vm/src/module.rs:2058` | `own_property_keys` | routed: the `with` attributes of `import()` are any object; switch to the `try_` form |
| `crates/ep9_vm/src/facts_export.rs:982` | `object::proto_of` | routed: `chain_depth` over a receiver's chain reads a proxy as a null prototype (the walk ends early); stop or refuse at a proxy |
| `crates/ep9_builtins/src/array.rs:330`, `crates/ep9_builtins/src/array.rs:518`, `crates/ep9_builtins/src/array.rs:748`, `crates/ep9_builtins/src/array.rs:1152`, `crates/ep9_builtins/src/json/stringify.rs:79`, `crates/ep9_builtins/src/json/stringify.rs:319`, `crates/ep9_builtins/src/json/parse.rs:80` | `Cx::is_array` (the frozen form: false for a proxy) | routed: IsArray's Proxy step (and its revoked-proxy TypeError) in ArraySpeciesCreate, `flat`, `Array.isArray`, IsConcatSpreadable, JSON.stringify's replacer and value, JSON.parse's reviver walk; switch to `Cx::is_array_or_throw` (H20 part 2, M4-P4) |
| `crates/ep9_builtins/src/json.rs:130` | `Cx::is_array` | `primitive_data`: a quick "not a wrapper" test; a proxy is no wrapper either, so false is right: stays (the 8th caller R-VM-P4-3 names) |
| `crates/ep9_vm/src/natives.rs:1430` | `is_extensible` | an Array by construction (`this.as_array()`): stays |
| `crates/ep9_vm/src/interp/slow/runtime.rs:135` | `set_prototype_of` | `__proto__` in an object literal: the literal is new and ordinary: stays |
| `crates/ep9_vm/src/module.rs:880` | `prevent_extensions` | the namespace object it just made: stays |
| `crates/ep9_vm/src/builtins.rs:1427`, `crates/ep9_vm/src/builtins.rs:1445` | `set_prototype_of` | intrinsic constructors: stay |
| `crates/ep9_vm/src/interp/ops.rs:926` | `object::proto_of` | `[[HomeObject]]` is ordinary; the lookup start may be a proxy, and the named IC sends a proxy to the generic path (`ic.rs` `exotic_name`, M4-P4 fixer B) |
| `crates/ep9_vm/src/interp/ops.rs:1691` | `object::proto_of` | the active function, never a proxy: stays |
| `crates/ep9_vm/src/abi/script.rs:246` | `object::proto_of` | the function just compiled: stays |
| `crates/ep9_vm/src/property.rs:117` (`get_data_from`) | the cell's storage | engine error text only, no JS: a proxy reads as empty; PROXY2 names the proxy in its doc |
| `crates/ep9_vm/src/property.rs:657` (`define_routed`) | the falsish-trap text | with no trap V8 gives the target's text ("Cannot define property 0, object is not extensible", R-VM-P4-6): PROXY2 picks the text by trap |
| `crates/ep9_vm/src/bound.rs:154` (`bound::unwrap`) | `[[Call]]` of a PROXY_CALLABLE | the call path takes a callable proxy for a bound function (debug assert; R-VM-P4-7); PROXY2 builds `[[Call]]` and `[[Construct]]` in `call.rs` |
| `crates/ep9_builtins/src/object.rs:245` (`isPrototypeOf`), `crates/ep9_builtins/src/object.rs:411` (`__lookupGetter__`, `__lookupSetter__`) | `Cx::prototype_walk_step` | a test gap, not a form (verifier B V-B3, M4-P4): no `ep9_builtins` test reaches the walk step through these builtins (`tests::proxy` calls the step directly; with object.rs put back to dba9e30 every `ep9_builtins` test passes). PROXY2 adds a JS test of each on a proxy cycle (`a` whose prototype is `new Proxy(a, {})`) next to node v24.22.1, which throws RangeError "Maximum call stack size exceeded" for each |
| `crates/ep9_vm/src/exotic.rs:185`, `crates/ep9_vm/src/exotic.rs:364`, `crates/ep9_vm/src/property.rs:1551` | ordinary `own_attrs`, `own_property_keys` | a release build answers "ordinary" for a proxy (the V9-A21 asserts are debug only); every caller that can meet one takes the `try_` form |

Done in M4-P4 and off this list: `Cx::has_index` asks `try_has_index` (Codex PROXY1 P-3); the named load and store ICs send a proxy to the generic path (R-VM-P4-1); the prototype walks of instanceof, for-in, `isPrototypeOf` and `__lookupGetter__` / `__lookupSetter__` are bounded and poll (Codex PROXY1 P-2), and proxy forwarding has a depth bound (P-1). PROXY2 also adds the trap lookup in `proxy.rs` `start` and repeats the MC-w15-12 line (row 3).

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
| H31-Q1 | The registry API: fuse id type, how code registers a dependency, how a pop finds dependent code and frames | M1 exit (API freeze) | Fuse = stable index (H13); a dependency list per fuse of weak code handles; a pop walks the list, marks code, then swaps return addresses (H29). **Proposal 2026-10-27 (M4-P2 ADR-A0; C-01, Q1 part; MODES-D3): FuseRef = Static(index) \| Dynamic(descriptor).** Static: a fuse with a stable index (H13 list; the builtin and debugger fuses), read through RootIndex or a unit link cell. Dynamic: a fuse made at run time per shape, field, class, enum or prototype, named by a descriptor and read through a LinkSlot whose link cell is resolved at load or first run (H34, H35); no unit code bakes a fuse-word address, and JIT code may use the immediate LinkSlot form that H34 allows at JIT finalize. Descriptor kinds form an append-only list (Freeze table). The first named kind is "prototype-chain validity by ProtoRef": the descriptor names the prototype (a ProtoRef, the H16 FactSnapshot form), not a VALIDITY_CELL, because an invalid cell stays invalid and the next IC miss makes a new one (`crates/ep9_vm/src/validity.rs:33-34`); the registry keeps the dependents. Dependencies are per isolate: a pop in any realm invalidates the dependent code object, and code that must survive a pop in another realm reads the fuse as a GuardFuse through the current realm. A dependency list per fuse holds weak code handles; a pop marks them Invalidated, lazily deopts their frames (H29) and rewrites each dispatch slot. The Rules row "Fuses have stable indices" changes with the fuse registry v0 in month 5, before M1a (C-01) |
| H31-Q2 | Fuse scope: per isolate or per realm. The builtin fuses guard realm intrinsics (H22) | M1 exit | Per realm for intrinsic fuses, per isolate for the debugger fuses |
| H31-Q3 | The fuse word format for a GuardFuse (where it lives, what "intact" reads as) | M1 exit | One word per fuse in the isolate root table, read through RootIndex (H13) or a unit link cell (H35) |
| H31-Q4 | Do F1-F6 and F8 of [R18] (per-key prototype conditions, HasPrototype, shape stability, field type, array protectors, buffer detach, closed world) join the list? They are for sound regions (A5v) | M7 exit (D17 rule) | Not part of the decision unless the D17 rule picks (d); F5 overlaps the "no-elements prototypes" and "species" entries |
| H31-Q5 | How T0 and Rust builtins (no compiled code) use a fuse. `proto_index` is a private flag today | M1 (registry) | Rust fast paths read the fuse word as a GuardFuse; `proto_index` becomes the "no-elements prototypes" fuse |
| H31-Q6 | README §3.4 asks a Freeze table: the API freezes at M1 exit, the registry is built until M2. Which parts freeze when? | M1 exit | API at M1 exit; the initial list at M2 exit. **Filled 2026-10-27 (M4-P2 ADR-A0; MODES-D1, C-01): the Freeze table below.** The initial list does not close at M2 exit: it is an append-only list with no close point, so a fuse added later (a new builtin protector) is a fill |

## Freeze table

README §3.4 form (H31-Q6; plan v9 §4.1 task 9; the design is [13](../plan/13-execution-modes.md) §3.5 and §5.4, MODES-D1). An open list works as H20 part 2: a new value is a fill when built, an existing value never changes, and G8 never counts an append. A negative test probes an unknown value, never the next free one.

| Part | What it holds | Freeze point | After the freeze point | Check |
|---|---|---|---|---|
| API | The two fuse kinds and their rules; FuseRef = Static \| Dynamic and the registry API (Q1); fuse scope (Q2); the GuardFuse word format (Q3); the T0 and Rust use (Q5) | M1 exit | Frozen-part rules (README §3.2) | fuse unit tests (registry v0, month 5); SP-Facts |
| Static fuse list (append-only) | The stable fuse indices (an H13 index list): the initial list of the Decision, in its order | None | Append only; an index never changes | the H13 stable-index order checks |
| Dynamic descriptor kinds (append-only) | v1: "prototype-chain validity by ProtoRef". Next by append when built: field constness and representation (H17, month 5), TS-derived class hierarchy and enum facts (A4) | None | Append only | an unknown descriptor kind -> the registry refuses it and the variant is skipped |
| A5v fuses (gated) | F1-F8 of [R18] (Q4) | The end of A5 (the K-A5v decision), only if the D17 rule picks (d) | Frozen-part rules | A5 gates |

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
| 2026-10-27 | fill (H31 is Draft) | Freeze table and Open-question text (plan v9 §4.1 tasks 2 and 9; MODES-D1, MODES-D3, C-01 Q1 part, MC-w13-11): the Freeze point field names the parts; Q1 proposal FuseRef = Static(index) \| Dynamic(descriptor), per-isolate dependencies, the first descriptor kind "prototype-chain validity by ProtoRef"; a Freeze table (the API at M1 exit; the static fuse list and the descriptor kinds as append-only lists; the A5v fuses at the end of A5); Q6 filled. The Rules row on stable indices stays for the month-5 fuse registry v0 (C-01). Every check keeps its result | M4-P2 ADR-A0 (VM lead role) |
| 2026-10-27 | clarify (H31 is Draft) | Review fix from the M4-P2 Claude reviewer (ADR-A0 scope, F6): the H31-Q1 proposal said no code bakes a fuse-word address, which conflicts with the H34 LinkSlot row for JIT finalize (an immediate is allowed); it now says no unit code does, and JIT code may use the immediate form. No check changes | M4-P2 integration fixer B |
| 2026-10-27 | clarify (H31 is Draft) | MODES-CHECK-w14 finding MC-w14-18: a table of the M4-P3 fast paths that assume a fuse, as KEYED and REGEXP1 name them in their reports (KEYED F4: `proto_index`, the v0 {w, e, c} element rule, the routed kinds list; REGEXP1: the `exec` fuse), each with the invariant, the fuse of the initial list (or none yet) and the test that stands in for it today. A fuse-guard op of the H16 CacheIR list comes by append when the registry lands (month 5). No check or list changes | M4-P3 integration fixer (docs part A) |
| 2026-10-27 | clarify | 1 `path:line` reference moved to the code after the M4-P3 merges (GC3, KEYED, IC2, REGEXP1, TOOLS) and the M4-P3 integration fixer's code commits (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS; the ADRLINE run of fixer part B, after its code fixes): `crates/ep9_vm/src/property.rs` 415-420 -> 435-440. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the new head). Text only; no check changes | M4-P3 integration fixer (part B) |
| 2026-10-27 | clarify (H31 is Draft) | 5 `path:line` references moved to the code after the M4-P4 merges (GC4, PROXY1, WEAKREF, MATH2, SPIKEC-4) and the M4-P4 integration fixer A's code commits (drift script, [I15]; M2-P4-FIX-ADRLINE-ROWS): `crates/ep9_vm/src/property.rs` 435-440 -> 461-466; `crates/ep9_vm/src/property.rs` 440 -> 466; `crates/ep9_vm/src/property.rs` 1440 -> 1638; `crates/ep9_vm/src/property.rs` 1032-1033 -> 1217-1218; `crates/ep9_vm/src/exotic.rs` 210 -> 238. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the new head). Text only; no check changes. Part B (M4-P4 integration fixer B, after its code commits edae2ba, cf6e643 and f29ca32; the same drift script): 4 more refs: `crates/ep9_vm/src/property.rs` 461-466 -> 469-474; `crates/ep9_vm/src/property.rs` 466 -> 474; `crates/ep9_vm/src/property.rs` 1638 -> 1726; `crates/ep9_vm/src/property.rs` 1217-1218 -> 1305-1306. Each names the same line text at dba9e30 and at the new head. Text only; no check changes. | M4-P4 integration fixer A; part B: M4-P4 integration fixer B |
| 2026-10-27 | clarify (H31 is Draft) | The PROXY2 caller list under the fast-path table (R-VM-P4-3 and Codex PROXY1 P-3, M4-P4 reviews): the callers of ordinary or proxy-blind forms that PROXY2 (month 5) must decide, each with its `path:line` at the M4-P4 head, among them the 8 `Cx::is_array` callers of `ep9_builtins`; and what M4-P4 fixer B already fixed. Text only; no check changes | M4-P4 integration fixer B |
| 2026-10-27 | clarify (H31 is Draft) | (1) The PROXY2 caller list gains a test-gap row (verifier B V-B3): `isPrototypeOf`, `__lookupGetter__` and `__lookupSetter__` on a proxy cycle, a JS test next to node for PROXY2. (2) 3 `path:line` references moved to the code after the M4-P4 fixer C code commits (`cargo xtask adr-lint` FIX lines): `crates/ep9_vm/src/property.rs` 1726 -> 1741, 1305-1306 -> 1320-1321, 1536 -> 1551 | M4-P4 integration fixer C |
| 2026-10-28 | clarify (H31 is Draft) | 3 `path:line` references moved to the code after the M5-P0 merges (P0-PLAT5, P0-VM5, GATES5, FUZZ-REC, CI5, DOC13) (`cargo xtask adr-lint --base m4-p5` FIX lines at 3c1c495; M2-P4-FIX-ADRLINE-ROWS): `crates/ep9_vm/src/shape.rs` 230-235 -> 273-278; `crates/ep9_vm/src/shape.rs` 1186-1187 -> 1230-1231; `crates/ep9_vm/src/shape.rs` 1186 -> 1230. Each ref names the same line text as before (both ends of each range read at the ref's intro commit and at the M5-P0 fixer head). The ref to the `GetNamedPropertyFromSuper` IC load (`interp/ops.rs` 926) keeps its number with a pin: P0-VM5 dropped the epoch argument from that line. Text only; no check, test or golden changes result (README §3.3 clarify). Review record (plan v10 rule 21a, §10): rows 1 and 3 the M5-P0 Claude reviewer (the fixer wrote this row), row 2 from the orchestrator's Codex texts, row 4 the fixer | M5-P0 integration fixer |

## Review record

Records of the Change-log rows from M5-P0 on (plan v10 rule 21a, §10: row 1 the phase's Claude reviewer, row 2 the phase fixer from the orchestrator's Codex texts, row 3 the fixer or the reviewer for the fixer's own rows, row 4 the fixer from the author's acceptance logs).

| Part (README §3.1) | Who | Result | Ref |
|---|---|---|---|
| 1 Reviewer (M5-P0 amendment: the 2026-10-28 ADRLINE clarify row) | M5-P0 Claude reviewer (Claude; plan v10 rule 21 (a) (c), §2.10; wrote none of the change: not this Change-log row, its ADR text or its pin lines (the M5-P0 integration fixer wrote them), no row 2 or 4 and no Codex text) | Checked at 726448c. (1) 3 moves: `crates/ep9_vm/src/shape.rs` 230-235 -> 273-278 (`proto_index` and its doc), 1186-1187 -> 1230-1231 and 1186 -> 1230 (`proto_index_keys`). Each range is the same text at 3c9f696 and at 726448c (every line, not only the ends), so the cell names the same code and its claim still holds. (2) Pin: `interp/ops.rs:926` keeps its number; at 3c9f696 that line passes `st.cx.proto_epoch` to `ic::load`, at HEAD it does not (IC_LAYOUT 3 item 4), so the line changed in place. The cell names `object::proto_of` as the lookup start, and that still holds. (3) The count (3) equals the body diff. (4) Kind test applied (README §3.3 clarify: 'Text only. No check, golden file, test or value changes result. The PR changes no code'): a2364ec changes `docs/` and lint data only (the drift pin table, the M2-P4-FIX-ADRLINE-ROWS form); no code, test, golden or value changes result -> clarify. Author: clarify (H31 is Draft). Confirmed (Draft: rule 21 (a) asks for the record). Open: none | branch m1/m5 at 726448c, mac a64 (`git show` of each moved range at 3c9f696 and at 726448c: scratchpad m1m5/p0/tmp/rev2-cmp.py, log rev2-cmp.log); adr-lint logs scratchpad m1m5/p0/tmp/rev2-adrlint-*.log |
| 3 Verifier (M5-P0 amendment: the 2026-10-28 ADRLINE clarify row) | M5-P0 Claude reviewer (Claude; wrote none of the change and none of rows 2 and 4. The M5-P0 integration fixer wrote this row, so this reviewer fills row 3 (plan v10 rule 21 (a), §2.10)) | Checked at 726448c. Row 2 is not written yet, so I read the Codex texts it will come from: M5-P0 Codex round 1, 8 slices (one per role branch, base 3c9f696). It has 15 findings (ci5 1, fuzzrec 2, gates5 5, plat5 2, pre 2, sbx 2, vm5 1; doc13 approve). None is in `docs/adr` or names H31, and no slice holds a2364ec (the fixer made it on m1/m5 after the merge 3c1c495). So there is no Codex finding on this row to sort. I checked the change myself instead: the 3 ranges are the same text at 3c9f696 and 726448c; ops.rs:926 changed only by the dropped epoch argument. Result: no finding; nothing to sort. Open (a)/(b): none. A later Codex round over 3c1c495..HEAD that names this row needs a row-3 addendum | branch m1/m5 at 726448c, mac a64; Codex texts scratchpad m1m5/p0/codex/r1-*.md; ref compare scratchpad m1m5/p0/tmp/rev2-cmp.py (log rev2-cmp.log) |
| 4 Data (M5-P0 amendment: the 2026-10-28 ADRLINE clarify row) | M5-P0 integration fixer (runs only; it wrote the change) | `cargo xtask adr-lint --base m4-p5` at 3c1c495: 92 FIX and 25 UNFIXABLE lines (path:line drift FAIL). A check of each FIX line (both ends of each range read at the ref's intro commit and at 3c1c495): 92 of 92 name the same text; 3 doc 13 refs into `interp/mod.rs` matched a repeated `#[cfg(...)]` line and were set by hand (doc 13 only). 3 FIX lines in dated record rows (H16, H20, H22) were not applied: history pins keep them. After a2364ec and 0942c03: `--base m4-p5` and `--base m0-exit` path:line drift PASS (checked=751 ok=751 (new=0) fixed=0 unfixable=0 pinned=14 history=112), short cites PASS (425 short cites, 0 findings). Text only: the commit changes `docs/` and the pin table, no code, test or golden | local runs; logs in scratchpad m1m5/p0/fixer1 (adrlint-m4p5-r0.log, adrlint-m4p5-r3.log, adrlint-m0exit-r3.log) |
