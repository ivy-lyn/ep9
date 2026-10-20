# ep9 plan: execution tiers and the shared optimizer

- The JIT tiers stay as in plan v1: T0 Rust interpreter, T1a/T1b baseline, T2 speculative mid tier, optional T3. T2 is the tier that matters [R06].
- The T1 and T2 emitters also build the two AOT code kinds: baseline-AOT (A3) and optimized-AOT (A4). One optimizer serves both modes, as in Dart, Graal, .NET, ART and Leyden [R11][R14].
- `ep9_masm` emits one instruction stream with abstract relocation kinds. Its JIT finalizer writes W^X code. Its AOT finalizer writes unit text with zero text relocations (H34).
- The IR keeps proven facts apart from predicted{source, confidence} facts. The guard-failure policy per mode lives in the IR, not in the backend (H30, H32).
- Fuses (H31), position-independent deopt metadata (H29) and FP rules (H36) are frozen by M1 exit, before any tier runs JS.

IDs used here: T0, T1a, T1b, T2, T3; baseline-AOT, optimized-AOT; JIT mode, AOT-jitless mode, hybrid mode; H2, H5, H6, H7, H8, H13, H14, H15, H16, H17, H20, H22, H24, H27, H29, H30, H31, H32, H33, H34, H35, H36; D9, D13, D16, D17, D21; G2-G4; N4, N5, N7, N11; M0-M9; A2, A3, A4, A5; RX0, RX1, RX2; FV1; K-A3, K-A4J, K-A4P, K-A4H; M5c; Spike B, SP-Unit, Spike C, SP-Facts, SP-Dispatch, SP-JIT-mem; PROVEN, RUNTIME, PROFILED, HINTED; DEOPT, SLOWPATH, PIN, THROW; GuardFuse, InvalidatingFuse; RootRef, IntrinsicRef, ShapeRef, CellRef, UnitConst; FactSnapshot; JITLESS-SET.

Evidence tags: [R03] [R05] [R06] [R07] [R08] [R09] [R10] [R11] [R12] [R14] [R15] [R16] [R17] [P1]. Paths are in the table at the end.

## 0. Reference numbers

All numbers come from the research machine (Apple M5 Max, 18 cores). Speed gates are ratios against reference engines run on the same machine and day, or against a same-host baseline (SP-Dispatch, Spike B).

| Engine / tier | Octane-7 geomean, x QuickJS [R06] | TypeScript workload, s [R06] |
|---|---|---|
| QuickJS 2026-06-04 | 1.00 | 4.30 |
| Brimstone (Rust, match loop) | 1.23 | 3.59 |
| V8 jitless | 1.33 | 2.80 |
| JSC LLInt | 1.48 | 3.32 |
| V8 + Sparkplug | 2.08 | 1.75 |
| JSC + Baseline | 3.90 | 1.06 |
| V8 + Maglev | 13.98 | 0.53 |
| JSC + DFG | 13.72 | 0.51 |
| V8 full | 25.62 | 0.55 |

v8-v7 scores per kernel, one run each; noise is ±10-20% [R07]:

| Kernel | QuickJS | node --jitless | Brimstone | Brimstone / jitless |
|---|---|---|---|---|
| Richards | 1880 | 2651 | 2923 | 1.10 |
| DeltaBlue | 1757 | 1273 | 1494 | 1.17 |
| Crypto | 2536 | 2160 | 2858 | 1.32 |
| RayTrace | 4561 | 7622 | 4712 | 0.62 |
| EarleyBoyer | 5347 | 11405 | 6861 | 0.60 |
| RegExp | 1046 | 7772 | 3651 | 0.47 |
| Splay | 9331 | 12924 | 12745 | 0.99 |
| NavierStokes | 5087 | 3402 | 4566 | 1.34 |
| **Score** | **3168** | **4644** | **4137** | **0.89** |

- Brimstone's geomean over the 7 kernels without RegExp is about 0.98x node --jitless (computed from the table).
- RayTrace and EarleyBoyer decide the M2 gate. RegExp has its own gate (RX1, due at M5 exit).

## 1. Tiers and code kinds

The T0, T1b and T2 targets are goals G2, G3 and G4 ([./02-architecture.md](./02-architecture.md)).

| Tier / code kind | Design | Frames | Octane-7 target, x QuickJS | TS workload | Compile cost | When |
|---|---|---|---|---|---|---|
| T0 Rust interpreter | Register + accumulator bytecode. DSL dispatch. Interpreted CacheIR-like ICs. Records all H16/H29 profiles | VM stack (D21) | ≥ 1.33 | ≤ 2.8 s | Bytecode generation only | M0-M2 |
| T1a baseline | Sparkplug style: one pass, shared stubs, compiled IC data | Native stack, canonical layout | ≥ 2.0 | ≤ 1.8 s | Template (raw emission 4.2 ns per bytecode) | M5 |
| T1b baseline | + inline ICs and int32 fast paths (JSC style) | Same | ≥ 3.5 | ≤ 1.1 s | Template | M5 |
| T2 mid tier | CFG SSA IR. FactSnapshot taken on the main thread, compile off-thread. Deopt and OSR | Native stack | ≥ 10 (aim 14) | ≤ 0.6 s | About 10x baseline (the Maglev ratio) | M7 |
| T3 top tier | Turboshaft-class passes on our IR, or hot T2 IR lowered to Cranelift | Native stack | ≥ 20 | n/a | High | M9 (optional, N4) |
| baseline-AOT | T1 emitter with the AOT finalizer (`ep9c build --baseline`). Data ICs write the FactSnapshot schema (H33) | Canonical frame | FV1: JITLESS-SET ≥ 1.6x T0 and ≥ 1.6x node --jitless (est.). Expect 1.6-2.8x T0 [R14] | ≤ 1.8 s (A3 gate, est.) | Build time | A3; gate at M8 exit (M5 exit if D16 = yes); K-A3 pre-check at M5 exit in every case |
| optimized-AOT | T2 (later T3) pipeline at build time on PROVEN, PROFILED and HINTED facts. Guards per H32 | Canonical frame | K-A4J: ≥ 1.5x baseline-AOT on JITLESS-SET (est.) | n/a | Build time | A4 with M7; gates by M8 exit |

```
             emitter crate      JIT mode (runtime, `jit`)      AOT (build time, ep9c, `aot`)
T0           ep9_vm (Rust)      always linked                  always linked: eval, new Function, deopt target (H5)
T1a / T1b    ep9_baseline  ---> T1 code                        baseline-AOT    (A3)
T2 (/T3)     ep9_opt       ---> T2 / T3 code                   optimized-AOT   (A4)
```

- [R17] lowers the bottom of the baseline-AOT range: Hermes untyped native gives 1.3-2.0x its interpreter. K-A3 fires below 1.3x T0 (§11).
- Naive AOT can lose to an existing JIT. Hermes untyped native was slower than Hermes' own baseline JIT on every kernel [R15][R17]. So baseline-AOT reuses T1's IC and fast-path code, not generic runtime calls.

## 2. T0: techniques that reach parity

| Technique | Evidence |
|---|---|
| Macro-DSL handlers. Debug builds use `match`. Release uses sibling calls, with a CI disassembly check and a deep-loop test. Nightly `become` is opt-in. At most 4 handler arguments (H15) | [R06]: sibling call 1.3-1.5x faster than match on dispatch-bound code; the debug sibling-call build overflowed the stack. [P1] (SP-Dispatch): a 5th argument breaks the tail call on Win64 |
| pc, frame base and accumulator are handler arguments. Cx carries the dispatch table at a fixed offset | [R06]; [P1] (SP-Dispatch) |
| Small fast paths. Slow paths are `#[cold] #[inline(never)]` runtime functions. T1, T2 and both AOT kinds call the same ones | [R06] |
| Scope analysis puts locals that are not captured in registers. Contexts hold only captured variables | [R07] Boa +25% |
| Lazy FeedbackVector. The interrupt budget at back-edges and returns drives tier-up, GC polls and termination | [R06] V8 waits 8 calls |
| Global variables use property cells plus ICs. Prototype validity cells guard proto-chain ICs | [R06] |
| Calls never recurse in Rust. Args are reversed and padded with undefined | [R06] adaptor frame: +11.2% jitless |
| Hot builtins (push, indexOf, string ops, Math) get human fast paths. Each has a differential test against its spec-layer path (N7) | [R07] |
| RegExp parity comes from RX1 (§3) | [R08] regress 0.8-5x slower than V8's regexp interpreter; [R07] RegExp kernel 7772 / 3651 / 1046 |

## 3. RegExp stages

| Stage | Engine | Milestone | Gate | Modes |
|---|---|---|---|---|
| RX0 | Fork of regress 0.12.0: Latin-1 cursor, interrupt hook, backtrack budget | M1 | built-ins/RegExp ≥ 99% (Boa reaches 99.8% with regress [R07]) | All, until RX1 |
| RX1 | Rust port of irregexp's parser, compiler and bytecode interpreter (BSD-3) | Starts M2; gate by M5 exit | v8-v7 RegExp kernel ≥ 0.8x node --jitless, in the jitless build. 0 diffs against RX0 on test262 RegExp | All. Top RegExp engine in AOT-jitless mode |
| RX2 | irregexp native backend on `ep9_masm`, plus a linear-time fallback for ReDoS | M5 | The 6 [R08] patterns run in ≤ 2x V8 JIT time | JIT mode and hybrid only (needs runtime codegen) |
| Never | The Rust `regex` crate | n/a | Nova passes 0/19 lookbehind tests with it [R08] | n/a |

- Early errors come from `oxc_regular_expression` in the parse path. It costs ≤ 1% of instructions [R03].
- The matcher compiles from its own parser on first exec. A differential fuzz target checks that both parsers agree.
- Build-time native regex for literal patterns comes after v1. Dart AOT also runs regexps on a bytecode interpreter [R11].
- D9: pure Rust, so no interim irregexp FFI.

## 4. Tier-up, dispatch slots and hybrid re-tier

**Starting thresholds** (perf CI tunes them) [R06]:
- FeedbackVector after 8 calls.
- T1 at about 500 warm-up units (JSC).
- T2 after about 400 calls; OSR after about 100 (Maglev values).
- Hybrid re-tier: an AOT function enters instrumented T1 after 1,000 calls (est.), tuned in A4. .NET re-JITs R2R code after 30 calls [R14].

**When to speculate.** Only at a high profiled probability. JSC uses 0.9994, because one exit costs 2.5-10 µs [R06]. Per-bytecode exit counters in RW data stop a failed speculation from repeating.

```
call f --> dispatch slot[f] = { kind, entry }        kind: interp | baseline | opt | aot-baseline | aot-opt
           tier-up, unit load, deopt-all, PIN  ==>  write one slot. Never patch a call site (H14, H16).
```

| Event | JIT mode | hybrid mode | AOT-jitless mode |
|---|---|---|---|
| Unit loaded | n/a | aot-* slots installed | aot-* slots installed |
| Function gets hot | T0 -> T1 -> T2 by thresholds | aot-opt -> instrumented T1 -> T2; aot-baseline -> T2 (its ICs wrote feedback) (H33) | Never re-tiers |
| Predicted-fact guard fails | DEOPT to T0 + exit counter | DEOPT, then instrumented re-tier | SLOWPATH, or DEOPT within budget, then PIN to aot-baseline |
| InvalidatingFuse fires | Lazy deopt + slot swap | Same | Lazy deopt + slot swap to aot-baseline or interp |

**Feedback rule (H33).** Optimized AOT code does not produce feedback for free; .NET never instruments R2R code [R11][R17].
- baseline-AOT ICs write the shared feedback schema.
- optimized-AOT writes only call and exit counters.
- Hybrid re-tiers hot optimized-AOT functions through instrumented T1, then T2.
- Runtime feedback overrides PROFILED and HINTED facts. A replayed profile can lock in slower code (Leyden Havlak: steady state 20-40% slower) [R11].
- K-A4P decides if hybrid ships ([./05-aot.md](./05-aot.md)).

## 5. JIT infrastructure and the relocatable MacroAssembler

- **Encoder.** `ep9_masm` on dynasmrt 5.1.0 for x64 and arm64. Own encoders come later: dynasm-rs has one maintainer and covers arm64 only up to ARMv8.4 [R06].
- **Executable memory** [R06][R09]:
  - macOS: `MAP_JIT`, `pthread_jit_write_protect_np`, `sys_icache_invalidate` (verified; a toggle pair costs about 23 ns).
  - Linux: dual mapping or mprotect.
  - Windows: VirtualAlloc plus `RtlAddGrowableFunctionTable` for unwind info.
- **Code range (H7).** One reservation holds JIT code and mmapped units. arm64 direct calls reach 128 MiB, and Apple M1 never predicts targets beyond 4 GiB [R11].
- **ICs (H16).** IC state lives only in RW data. No code patching in JIT or AOT code. An optional embedded stub corpus serves units; weval uses 2,367 precompiled CacheIR stubs [R14].
- **Store barrier in emitted code (H11).** Barrier elision follows H11 rules E1-E3 only; E2 ends at the first escape or GC point; no barrier clustering in v1. No GC point sits between the barrier check and the store, and a loaded threshold or gc_state is never reused across a GC point ([03 §3](./03-gc.md#3-barrier-one-fast-path-in-every-tier-and-code-kind)).
- **Stack support.** Frame walker, exception unwinding, perf jitdump, and a perf map for units (H24). The pc->code registry covers JIT code and unit ranges (H14). JIT and AOT frames are scanned conservatively while the heap does not move [R05] ([./03-gc.md](./03-gc.md)).
- **Features (H27).** `jit` = runtime codegen. `aot` = unit emission. The jitless build has no `jit` but keeps T0, the bytecode compiler, the `ep9_unit` loader and the deoptimizer.

**One instruction stream, two finalizers (H34).**

```
ep9_baseline / ep9_opt
      | one instruction stream + abstract relocations
      v
  ep9_masm
      +-- finalize_jit  (`jit`) --> W^X code space, absolute addresses allowed,
      |                             dispatch-slot write, jitdump
      +-- finalize_unit (`aot`) --> unit text (RX, zero text relocations) + RW link table
                                    + RW IC cells + PI deopt metadata --> ep9_unit writer (.o / .ep9u)
```

| Kind | Meaning | JIT finalize | AOT finalize |
|---|---|---|---|
| Local | Label or branch inside the function or unit | pc-relative, resolved at emit | pc-relative, resolved at emit |
| Stub(id) | Shared IC stub or trampoline | Direct call inside the code range | pc-relative call if the stub is in the unit; else call through a link slot |
| Runtime(fn) | Rust slow path or builtin (H20), e.g. barrier_slow | Direct or indirect call | Indirect call through the isolate table (isolate register) |
| LinkSlot(i) | External reference resolved at load or first run | Resolved at finalize; an immediate is allowed | pc-relative load of RW link cell i |
| IcCell(i) | IC or feedback cell | Address of the FeedbackVector slot | pc-relative load of RW IC cell i |
| RootIndex(n) | Isolate root, intrinsic or atom by stable index (H13) | Load from the isolate root table | Load from the isolate root table (isolate register) |
| DeoptExit(id) | Exit to the shared deopt entry | Call the shared entry; metadata keyed by pc offset | Same; metadata keyed by pc offset |
| HeapConst(i) | Raw heap pointer | Embedded, held weakly with a dependency | **Rejected by the unit writer** |

- The AOT finalizer must give zero text relocations. An xtask check enforces it. Wasmtime refuses precompiled code with relocations outside `.debug_*` [R14].
- The IR has no raw heap constants (H30), so the HeapConst rejection is a backstop.
- No LLVM in JIT or AOT code. Cranelift only for the optional T3, shared by JIT and AOT emit (N5 refined) [R06][R14].
- [./05-aot.md](./05-aot.md) owns the unit container, header fields and load flow (H35).

**Spikes.**
- **Spike B (M0, month 2).** A 10-opcode template JIT measures the T0 -> JIT -> T0 round trip on each target.
  - Keep native-stack JIT frames if one round trip costs ≤ 18 sibling-call dispatches on the same host (est.). Otherwise open an ADR for one unified VM stack.
  - It also settles D21 (interpreter frame home; default: VM register stack + native control stack, Hermes model) [R14].
- **SP-Unit (inside Spike B, +0.25 PM est.).** The same 10 opcodes go into a unit blob. It must load, call, unwind, survive a GC scan and fall back on a version mismatch. Gate: green on 4 targets.
- **Spike C (M1, throwaway, 1.5-3 PM est.).** A small SSA compiler for 2-3 v8-v7 kernels on arm64. It reads T0 feedback and has one eager deopt from a native frame to a VM-stack frame. Gate: deopt correct under the tier-differential test; kernels ≥ 3x faster than T0 (est.).
- **SP-Facts (inside Spike C, +0.25-0.5 PM est.).** The Spike C IR uses the fact lattice and ref nodes (§6) and emits PI deopt metadata (§9). Gate: one kernel compiled from a PROFILED FactSnapshot file gives identical output at ≥ 90% of live-feedback speed (est.).
- A failed Spike C or SP-Facts gate reopens H2, H14, H16, H17 and H29-H34 before M1 exit.
- SP-Unit and SP-Facts have a 2-week time box (est.) beyond their budget. Correctness and structure failures still reopen the ADRs. A speed-only miss after the time box is recorded in the H30 ADR and does not block the exit ([./09-milestones.md](./09-milestones.md) §4).

**No JIT tier code before the M2 gate.**
- Spikes B and C, SP-Unit, SP-Facts and SP-JIT-mem are throwaway.
- After M1 exit, tier-agnostic JIT infra may start behind `jit`, and the unit writer/loader (A2) behind `aot`. Neither may contain a tier that runs JS.
- LibJS removed its 2023 JIT because its interpreter was 5x slower than JSC's [R07]. The lesson is about shipping a tier on a slow base.

## 6. Shared optimizer: the IR fact model (H30)

```
  unknown            generic code path, no guard
     |
  predicted{source, confidence}     source: RUNTIME | PROFILED | HINTED
     |                              needs a Guard; precedence RUNTIME > PROFILED > HINTED
  proven                            no guard
                                    (proven ⊑ predicted ⊑ unknown)
```

**PROVEN producers** (facts ep9 proves from JS semantics) [R14][R16]:
- module bindings that are never reassigned (ESM graph), and `const` bindings;
- scopes with no direct eval and no `with`;
- bitwise results are int32; own range analysis;
- spec invariants (for example, `typeof` gives a string).

TS types are never PROVEN in full-ECMAScript mode. They enter only as HINTED facts behind guards (N11) [R12][R17].

- A debugger can write locals and bindings. The H24 rule keeps PROVEN facts sound: local writes go only to T0 frames, and binding writes pop the DebuggerBindingWrite fuse [R18].
- While an inspector session is connected, T2 and AOT code treat every interrupt poll as a heap effect point (H24); a session can run JS there and retype a field inside a call-free loop [R18].
- Math range facts may assume CR results, for example `|sin x| <= 1`, `exp x >= 0`, monotonic `log` and `exp` (H36).
- **VERIFIED (candidate, D17 d).** Facts that the `ep9c` verifier proves per region, under entry checks, hoisted interior checks and fuses. Built only if the D17 rule picks (d) at M7 exit; until then H30 reserves the provenance value [R18].

**IR nodes with no raw heap constants.**

| Node | Meaning | Lowers to |
|---|---|---|
| RootRef(n) | Isolate root by stable index (H13) | RootIndex(n) |
| IntrinsicRef(j) | Realm intrinsic by stable index | Load from the current realm's intrinsics table (H13, H22) |
| ShapeRef(s) | A shape used in a guard | ShapeId immediate only for snapshot root shapes; else a CellRef (H8) |
| CellRef(i) | RW IC or link cell | IcCell(i) or LinkSlot(i) |
| UnitConst(i) | Constant from the unit's constant pool | pc-relative rodata load |
| Guard(kind, fact) | Tag, shape, elements kind, call target or non-null check | Policy from H32 per mode (§7) |
| Assume(fuse) | Global invariant | GuardFuse check or InvalidatingFuse dependency (§8) |

**One graph builder, Warp style** [R06][R14][R17]:

```
JIT:  FeedbackVector --(snapshot on main thread)--> FactSnapshot{RUNTIME} --+ merge (H30) --+
      CompiledUnit hint section (only if K-A4H keeps it) ---> HINTED -------+               |
AOT:  .ep9prof (training run) ---> PROFILED -+                                              |
      .ep9h (ep9-typecheck) -----> HINTED ---+--> ep9_aot merge --> FactSnapshot -----------+--> graph builder
      module graph ---------------> PROVEN --+                                                (bytecode + FactSnapshot)
                                                                                            --> SSA CFG --> passes --> ep9_masm
```

- The FactSnapshot schema equals the feedback schema (H16). So runtime feedback, profiles and hints are interchangeable inputs.
- **Merge rule (H30).** Precedence applies per fact kind. On a conflict, RUNTIME wins, so a slot with feedback ignores HINTED shape, call-target and representation facts. Fused HINTED facts that feedback cannot give stay: CHA and enum constants (H31), non-null, slot pre-size ([./05-aot.md](./05-aot.md) §11).
- Dart keeps about 276 AOT branches in about 237k lines of one compiler [R14]. ep9 expects the same pattern: few mode branches, all in guard lowering and fact sources.

## 7. Guard-failure policy per mode (H32)

| Fact | JIT mode | hybrid mode | AOT-jitless mode |
|---|---|---|---|
| PROVEN | No guard | No guard | No guard |
| RUNTIME | Guard -> DEOPT + exit counter | Same | n/a (no runtime compiler) |
| PROFILED | Guard -> DEOPT | Guard -> DEOPT -> instrumented re-tier (H33) | Guard -> SLOWPATH, or DEOPT within budget, then PIN to baseline-AOT |
| HINTED | As PROFILED, at lower confidence | As PROFILED | As PROFILED |
| Global invariant | InvalidatingFuse: lazy deopt + dispatch-slot swap | Same | GuardFuse (load + compare) or InvalidatingFuse (lazy deopt patches no code) |
| SOUND module boundary (A5 only) | THROW | THROW | THROW |

- Never THROW on a hint failure. THROW exists only in SOUND modules at their boundaries, outside test262 mode (A5, only if the D17 rule picks (c)).
- VERIFIED regions (A5, only if the D17 rule picks (d)) never THROW: an entry-check failure takes SLOWPATH from the start, an interior-check failure takes DEOPT with frame state, and a fuse pop takes lazy deopt + slot swap (H32 reserved rows) [R18].
- No deopt loops in jitless mode. ART emits no deopt in AOT code because of "forever deoptimizations" [R11][R14].
- Budgets: 4 failures per site -> SLOWPATH, 16 exits per function -> PIN (est.). [./05-aot.md](./05-aot.md) owns them.

## 8. Fuses (H31)

| Kind | In the code | On invalidation | Fast-path cost | Main use |
|---|---|---|---|---|
| GuardFuse | Explicit load + compare of a fuse word (RootIndex or link cell) | Code sees the new value and takes the slow path | One load + branch | AOT-jitless; any code that must not deopt |
| InvalidatingFuse | Nothing; a registered dependency | Lazy deopt of dependent frames (return-address swap) + dispatch-slot swap | Zero | JIT mode, hybrid; jitless where another entry exists |

- **Initial list, builtins:** Array species; Array, String, Map and Set iterators; Promise.then lookup; no-elements prototypes; RegExp exec; TypedArray species.
- **Initial list, other:** shape constness and replacement watchpoints (H17); TS-derived class-hierarchy and enum facts (A4); `InspectorSession` (pops on session connect) and `DebuggerBindingWrite` (pops before a debugger writes a binding) (H24) [R18].
- Fuses are one-way: intact, then popped. V8 has 22 one-way protectors. SpiderMonkey splits GuardFuse and InvalidatingFuse. JSC uses WatchpointSets [R14].
- Enum objects are mutable at runtime, so enum folds need a fuse [R16].
- The registry lives in `ep9_vm`. API frozen at M1 exit; built in M1-M2.

## 9. Deopt contract (H29)

| Per-exit field | Content |
|---|---|
| pc offset | From the code start, so the metadata is position-independent |
| Bytecode offset | The only deopt and OSR identity (H2) |
| Value locations | Every live register and the accumulator: reg, stack, const-index or materialize |
| Inline frame chain | Function id + bytecode offset for each inlined frame |
| Exit counter | Index of a counter in RW data |

```
eager: Guard fails -> DeoptExit(id) -> shared deopt entry (ep9_vm)
         -> pc->code registry -> metadata at pc offset -> rebuild T0 frames (MovHint style, inline chain)
         -> bump RW exit counter -> resume T0 at the bytecode offset
lazy:  fuse pops -> mark dependent code -> swap return addresses on the stack -> frames enter the deopt entry on return
```

- Lazy deopt swaps return addresses only. It never patches code, so it works on read-only, signed unit text [R14].
- Every interrupt poll (loop back-edge, function entry) in T1, T2 and AOT code is a lazy-deopt point with full metadata (H29), so a fuse popped at a poll can deopt the frame [R18].
- TurboFan code has 12.5-17.1 deopt points per KB [R14]. So this metadata format sets much of the unit size.
- The deoptimizer runtime lives in `ep9_vm`, not `ep9_opt`. The jitless build keeps it (H27).

## 10. Floating-point rules (H36)

- No FMA contraction in any backend. `ep9_masm` has no FMA encoder. Rust code uses no `mul_add` and no `algebraic_*` outside `ep9_math::exact`.
- Exact IEEE-754 results in every tier and code kind. ToInt32 is `ep9_value::to_int32`; arm64 emitters may use FJCVTZS.
- One Rust Math: crate `ep9_math`. It serves T0-T3, baseline-AOT, optimized-AOT, BytecodeGen folds and `ep9c`. No tier, compiler or dependency calls the platform libm for a JS-visible value.
- Target: the correctly rounded (CR) result for `sin cos tan asin acos atan atan2 sinh cosh tanh asinh acosh atanh exp expm1 log log1p log2 log10 cbrt pow`. A CR result is unique, so it does not depend on the platform, the compiler or the code. V8 15.8 already gives the CR result for 13 of these.
- `**` and `Math.pow` share `ep9_math::pow_js`: JS special cases, then CR pow. `Math.hypot` ports V8's algorithm. JS `%` uses `ep9_math::fmod`, never C `fmod`.
- Emitters inline only a closed list: `sqrt`, `abs`, `fround`, `floor`, `ceil`, `trunc`, `round`, `sign`, `min`, `max`, `clz32`, `imul`, ToInt32, and `**` with a constant exponent of ±0, 1, 2, -1 or 0.5. Other Math calls go to `ep9_math` through the external-reference table.
- Default FP environment only: round-to-nearest-even, no FTZ/DAZ. NaN purification on boxing (H6).
- Lint, every PR: root `clippy.toml` bans std libm methods, `powi`, `rem_euclid`, `div_euclid`, `mul_add` and `algebraic_*`. `cargo xtask fp-check` fails on a banned libm symbol, an FMA instruction outside `ep9_math`, or a second math crate, on all 5 targets. Exceptions live in `expectations/fp-check-allow.toml`.
- Tests: `ep9_math` golden bits and 1M-input hashes on 4 targets and on x86-64 with and without FMA. Nightly CR oracle (mpmath) and d8 comparison with ULP tolerances. FP differential CI from M1.
- nbody and raytracer checksums must equal V8: nbody -0.169193895234998, raytracer 576888 [R15].
- Why: Porffor's C backend let clang contract `a*b+c` into an FMA; its nbody printed -0.16919383316763523 [R10][R15]. The same V8 13.6 Math source gives different results in node's macOS, Linux arm64 and Linux x64 builds, and V8 15.8 calls the host `pow` and `tanh` ([H36](../adr/H36-fp-semantics.md)).

## 11. Why this tier order

- **Why keep T1.**
  - Without a mid tier, T1 gives 1.6-3x on the TS workload [R06]. Ignition 2.74 s -> Sparkplug 1.75 s; LLInt 3.32 s -> JSC Baseline 1.06 s.
  - ep9 has no T2 for the whole M5-M7 window (120-200 PM of CORE work, est.).
  - With a mid tier present, T1 adds little on this workload. Maglev with Sparkplug ran 0.64 s, without it 0.62 s (plan v1 local re-run).
  - The gain on cold code (startup, code below T2 thresholds) is not measured.
  - T1 proves JIT memory, unwinding, Win64 SEH and tier fuzzing before the risky tier. V8's baseline is about 0.4 MB of source; Maglev is 4.4 MB [R06].
  - **New:** T1's emitter is also the A3 baseline-AOT emitter. Skipping T1 (D13) removes baseline-AOT, so jitless gains wait for A4.
- **Why the mid tier matters most.** Maglev and DFG reach 54-55% of full V8 on Octane. They match full V8/JSC on the TS workload [R06].
- **Why not Cranelift for T1/T2.** It costs 11-21 µs per bytecode, 2,600-5,000x template emission. It has no deopt value-location stack maps [R06].
- **Why not LLVM.** JSC's FTL left LLVM because LLVM took more than 75% of compile time [R06]. An LLVM or C AOT backend also brings FMA risk and a C++ toolchain (D9).
- **Optional M5c: a generated baseline interpreter** on the native stack, built by the T1 code generator. SpiderMonkey went from 31 to 52 on Speedometer with this [R06]. Two triggers:
  1. Profiles show T0 <-> JIT transitions cost > 5% (est.).
  2. K-A3 fires: baseline-AOT < 1.3x T0 on JITLESS-SET. Then M5c is emitted at engine build time and embedded as the jitless top tier, like V8 embedded builtins [R11][R14] (H15). The K-A3 pre-check runs at M5 exit in every case, so the AOT lane can build M5c in the M7 window (3-6 PM est.).
- T0 stays the portable, jitless reference in every case.

## 12. JIT and unit security

- W^X discipline and a write-protected dispatch table.
- Differential fuzzing across T0, T1, T2, baseline-AOT and optimized-AOT, plus Fuzzilli. Budgets are in [./10-quality.md](./10-quality.md).
- Hint chaos: random wrong hints and stale profiles must never change program output. Guards are mandatory (H32, N11) [R12].
- Units: RX-only text, content hash checked at load, optional signature. Units are trusted code, like native addons (D6).
- cargo-fuzz targets for the `ep9_unit` loader and the `.ep9h` and `.ep9prof` parsers.
- A named security owner from M5. The scope adds the unit loader from A2.
- Why: about 45% of V8 CVEs were JIT bugs [R07]. TS types are unsound by design, so an unguarded hint is a type-confusion bug [R12].

## Changes vs plan v1

- Crates renamed: `ep9_jit_baseline` -> `ep9_baseline`, `ep9_jit_opt` -> `ep9_opt`. The deoptimizer runtime moves to `ep9_vm`.
- Tiers table adds baseline-AOT (A3) and optimized-AOT (A4). RegExp stages renamed RX0-RX2; RX1 is the AOT-jitless engine.
- New: relocation kinds (H34), IR fact model (H30), per-mode guard table (H32), fuse registry (H31), PI deopt metadata (H29), FP rules (H36), aot dispatch kinds and hybrid re-tier (H14, H33).
- Spike B adds SP-Unit. Spike C adds SP-Facts; both are time-boxed. Unit infra (A2) may start after M1 exit behind `aot`.
- The H30 merge rule: RUNTIME wins on a conflict; fused HINTED facts with no runtime counterpart stay.
- M5c gets a second trigger: K-A3. T1 is also the A3 emitter (D13 note).
- Security adds units, hint chaos and loader fuzzing.
- Unchanged: tier targets, T0 techniques, thresholds, tier-order reasoning, Cranelift and LLVM verdicts for T1/T2.
- Decisions 2026-10-20: H36 FP rules rewritten (CR target, `ep9_math`, closed inline list, `clippy.toml` + `xtask fp-check`); H11 emitter rule and elision E1-E3; H24/H29/H31 debugger, interrupt-poll and fuse rules; D17 (d) VERIFIED candidate [R18].

## Evidence tags

| Tag | Path |
|---|---|
| [R03] | [../research/03-parser-perf-and-features.md](../research/03-parser-perf-and-features.md) |
| [R05] | [../research/05-gc-design.md](../research/05-gc-design.md) |
| [R06] | [../research/06-execution-tiers.md](../research/06-execution-tiers.md) |
| [R07] | [../research/07-rust-engine-landscape.md](../research/07-rust-engine-landscape.md) |
| [R08] | [../research/08-spec-subsystems.md](../research/08-spec-subsystems.md) |
| [R09] | [../research/09-critic.md](../research/09-critic.md) |
| [R10] | [../research/10-js-aot-precedents.md](../research/10-js-aot-precedents.md) |
| [R11] | [../research/11-dual-mode-vms.md](../research/11-dual-mode-vms.md) |
| [R12] | [../research/12-ts-type-soundness.md](../research/12-ts-type-soundness.md) |
| [R14] | [../research/14-dual-mode-architecture.md](../research/14-dual-mode-architecture.md) |
| [R15] | [../research/15-aot-perf-reality.md](../research/15-aot-perf-reality.md) |
| [R16] | [../research/16-aot-synthesis.md](../research/16-aot-synthesis.md) |
| [R17] | [../research/17-aot-refutation.md](../research/17-aot-refutation.md) |
| [R18] | [../research/18-sound-regions.md](../research/18-sound-regions.md) |
| [P1] | Plan v1 local probe (session-local); re-checked by the named M0 spike (here SP-Dispatch) |
