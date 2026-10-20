# AOT + JIT synthesis

## Short answer
Yes, ep9 can be one engine with both modes. But the evidence does not support "AOT with TS types is faster than V8" as a general goal.
- Sound-typed JS AOT (Static Hermes) ran 1.5-3.2x slower than V8 full on this Mac (geomean about 2.1x).
- Untyped AOT ran at baseline-JIT speed: 10-13x slower than V8, and slower than Hermes' own JIT.

Only one code shape beat V8: numeric code over fixed-layout typed classes with unboxed doubles. scriptc ran nbody 1.15-1.4x faster than V8. C that keeps JS float rules is only 1.22x faster, so that is the ceiling.

TS types are unsound: 24 of 24 strict programs gave wrong types at runtime, and 13 of 13 tsrs hint holes gave a wrong hint. Types prove int32 for only 1-9% of numeric expressions. They cannot prevent the main JIT failure, 'wrong map' (49% of deopts). So TS types can only be guarded hints, worth about 0-5% at peak.

Every production dual-mode VM (Dart, Graal, .NET, ART, HotSpot Leyden, Hermes) uses ONE optimizer with different fact sources. In all of them the JIT owns peak speed. AOT owns startup, memory, predictable speed and no-JIT platforms.

Recommended reframe:
- JIT for peak speed and a small app payload.
- AOT for startup (1.5-5 ms vs node 23.5 ms), memory (10-17 MB vs 50-62 MB) and iOS.
- TS types as free, guarded feedback for both modes.

"Faster than V8" is credible only in three cases:
- An opt-in sound typed module mode on numeric, monomorphic class code (about 1.1-1.4x).
- Megamorphic class dispatch in a closed world. Dart AOT takes 0.50-0.69x of JIT time; this is not shown for JS.
- Runs shorter than V8's 2-3 ms warm-up.

## Where AOT wins
- Startup. Hello world: Porffor 1.47 ms, QuickJS 1.99 ms, Hermes bytecode 4.03 ms, Static Hermes native 4.87 ms, vs node 23.5 ms and bun 5.77 ms (aot-vs-jit/startup.json). Loading 9.1 MB of TypeScript: Hermes .hbc 14.9 ms vs 836 ms from source; qjsc 37 ms vs 380 ms (aot-precedents/startup-results.txt). Most of this gain comes from serialized bytecode and snapshots, not from native code. Mikov credits caching for native tsc's 18 ms vs 75 ms startup.
- Memory. Peak RSS: Static Hermes 10-17 MB vs node 50-62 MB (results.json). native-image 57 MiB vs HotSpot C2 870 MiB, but the heap defaults differ (awfy_summary.txt). Hopc uses at least 3x less memory than V8 (ICFP'21). With no feedback vectors, recompiles or compiler threads, AOT saves memory.
- Platforms that forbid JIT (third-party iOS apps, W^X-strict hosts). There, AOT is the top tier. Static Hermes typed native runs Richards 13.4x faster than QuickJS. Untyped AOT gives 1.6-2.0x its interpreter (Hermes) or 2.77x (SpiderMonkey weval).
- Numeric code over fixed-layout, soundly typed classes with unboxed f64 fields. scriptc nbody 157 ms vs V8 196-223 ms (1.15-1.4x faster, bit-identical output). Static Hermes typed native beats V8 capped at Maglev on nbody (173 vs 246 ms) and ties it on raytracer (148 vs 152 ms). The ceiling is low: C with -ffp-contract=off is only 1.22x faster than V8 (74 vs 90 ms; bound.log).
- Megamorphic call sites in a closed world. Dart's global dispatch table makes AOT faster than JIT: TypedDataPoly.mega AOT/JIT 0.50, Iterators.poly 0.69 (dart_summary.txt). This is not measured for JS. In JS it would need fuses on prototype mutation.
- Short runs and predictable latency. V8 needs 2-3 ms to reach peak on kernels, a total excess of 1.4-4 ms per process (warm.log). AOT's first iteration is close to its steady state: native-image 0.9-1.4x, vs C2 at 2.07x (awfy). Typed AOT beats V8 only while the total hot runtime stays under about 3 ms.
- No JIT compile CPU. V8 spent 1.58 s of background compile CPU inside a 1.62 s tsc run (3,608 Maglev and 884 TurboFan compiles; trace-opt.txt). Wall time barely changes when spare cores exist, but CPU cost and energy fall.
- Closed programs with a training profile (Java evidence, not JS). Graal native-image with PGO ran at 0.89-0.92x C2's time on AWFY, and .NET R2R plus Leyden JEP 544 cut startup 65-80%. A training profile is a best case: in the AWFY runs it came from the same benchmarks.

## Where JIT wins
- Peak speed on general, OO and dispatch-heavy JS. V8 full is 3.16x faster than Static Hermes typed native on Richards (91 vs 288 ms) and 1.92x faster on nbody. On untyped Octane, Static Hermes native scores 0.094 of V8 and Porffor 0.078 (octane-table.md). On a real program (Meta, 2026-10-09), untyped native tsc took 1.20 s vs 354 ms on node.
- Call sites that are monomorphic or low-polymorphic in practice but not provably so. Dart TypedDataPoly with 2-5 receiver types: AOT/JIT 1.94 (up to 3.62). Dart Richards: 1.19-1.28. .NET NativeAOT is 2.1-5.3x slower than tiered PGO on Dictionary code (dotnet/runtime#129895). Static devirtualization reaches only about 15% of virtual and 5% of interface call sites, even with sound nominal types (.NET GuardedDevirtualization.md).
- int32 vs double. TS `number` never separates them, and types prove int32 for only 1-9% of non-constant numeric expressions. A JIT learns Smi/int32 from feedback, and that speculation almost never fails: 'not a Smi' was 0.4% of tsc deopts (deopt.log).
- Object shapes. 'wrong map' was 49% of V8 deopts on tsc. Interface types give no layout: class receivers are 0.4% of member accesses in the TS compiler and 6.4% in zod. Runtime ICs see the real shapes.
- Untyped JS and npm dependencies (Amdahl's law). Untyped AOT is 10-18x slower than V8. scriptc on plain JS Richards is 107x slower than V8 and 2.9x slower than QuickJS. checkJs types for JS are 30-39% `any`.
- eval, new Function, regexp and other dynamic code. Dart AOT runs regexps in an interpreter. Hopc 'cannot deliver good performance for programs relying on eval'. 1,791 test262 files use eval or Function(.
- App payload size. Native code is 6-19x the bytecode bytes: Sparkplug 6.9x, Maglev 11.6-19x, TurboFan 11.9-28.6x, Static Hermes untyped 10-14x and typed 3.4-4.7x. A JIT compiles only what runs: V8 lazy bytecode was 246 KB vs 364 KB eager, and 180 of 1,010 functions reached TurboFan. Static Hermes native tsc grew 26.3 to 69.8 MB.
- Long-running servers. Warm-up is paid once (2-3 ms on kernels, about 0.4-0.5 s on a 1.6 s tsc run), then JIT peak wins. Leyden's notes: 'minimizing JIT compilation often leads to lower peak performance'. A replayed training profile made Havlak's steady state 20-40% slower in my JDK 27 run.
- Adapting to runtime behavior. Feedback-only Warp beat SpiderMonkey's global type inference on real workloads: +10-12% Speedometer, -8% memory. V8 even added speculative inlining with deopt to statically typed Wasm: 1.59x on Dart micro-benchmarks, 1-8% on apps.

## Type trust model
RECOMMENDATION: three trust levels. The default for TS types is HINT (guarded speculation), never PROOF.

```
level      what goes in                               guard?                         on failure
PROVEN     facts ep9 proves from JS semantics:        none                           -
           module bindings, const, no direct eval
           in scope, bitwise result is int32,
           own range analysis
HINT       tsrs types via side table: value repr,     tag test / shape check /       JIT & hybrid: deopt + exit counter
(default)  class receiver -> shape + slot candidate,  null check / elements-kind     jitless: inline slow path, or deopt
           call target, non-null, element type,       check hoisted to loop entry    within an exit budget, then pin the
           enum constant, field types                                                function to baseline-AOT. NEVER throw.
SOUND      opt-in per module, later (A5): nominal     only at module, any, .d.ts     TypeError at the boundary (changes JS
(opt-in)   classes, exact fields, final methods,      and JS boundaries              semantics, so outside test262 mode)
           checked casts, bounds-checked arrays,
           no Proxy/eval/defineProperty on typed objs
```

WHY HINTS, NOT FACTS (evidence):
1. TS is unsound by design.
   - 24 of 24 strict programs pass tsc 6.0.3, tsc 7.0.2 and tsrs 0.18 with 0 errors, then produce 47 values that contradict their types at runtime (ts-type-trust/unsound/run.log; I recounted the 47 'LIE' lines).
   - The strictest flags close only 3 of the 24.
   - On the oracle side, 13 of 13 tsrs hint holes give a wrong hint at runtime (tsrs-type-oracle corpus/holes).
   - Porffor trusts types without checks and prints 17 where JS prints 'a1'.
2. Types carry little codegen information on real code.
   - int32 types: 1-9% of numeric expressions (TS compiler 37%, from unsound enums).
   - Class receivers: 0.4% (TS compiler) to 61% (pixijs).
   - Casts: 3-42 per KLOC.
   - pixijs runs with strictNullChecks off.
3. The guard-removal ceiling at peak is small.
   - Removing all V8 deopt checks gave 2.2-4.6% (IISWC'16); an IISWC'21 abstract puts checks at about 8%.
   - StrongScript's sound types inside V8 gave -0.9% to +22%.
4. Sound mode has a cost. Safe TypeScript checks cost 6.5-15%. Static Hermes and scriptc change semantics (throwing casts, out-of-bounds throws) that test262 forbids.

WHERE HINTS SHOULD PAY (to be measured; none of this is measured in a JS engine yet):
- (a) Pre-fill feedback for cold sites. 33% of tsc deopts on V8 were 'insufficient feedback', and that is the only deopt class that types ahead of time can address.
- (b) Class-heavy code. Use a shape guard plus a slot candidate: tsrs found the slot for 64% of member accesses on the typed AWFY port.
- (c) Call target plus class-hierarchy analysis, under a prototype fuse.
- (d) Enum member folding (32% of member accesses in the TS compiler), under a fuse, because enum objects are mutable at runtime.
- (e) Seed the field-type lattice and pre-size in-object slots.

Correction to one report: V8's 5.7x `x: number;` class-field cliff (517 vs 90 ms) comes from V8's HeapNumber field representation. JSC (NaN-boxed) shows no cliff (112 vs 114 ms). ep9 plans JSC-style NaN-boxing, so field-type hints buy less in ep9 than in V8. Mostly they help slot pre-sizing and tag-check removal behind guards.

DEMOTE OR DROP HINTS:
- from .d.ts and lib declarations (claims about foreign code; 65% of Preact member accesses);
- in files with type errors;
- after any, as, or `!`;
- on structural interface receivers (use those only to pre-size ICs).

Runtime feedback always wins over a hint. It is authoritative for int32 vs f64, shapes, call targets, holey vs packed, and constant fields. A site with N guard failures stops using its hint.

SEMANTIC INPUTS THAT CHANGE SHAPES: useDefineForClassFields gives [[Set]] vs [[Define]], and 4 of 6 measured codebases compile with [[Set]]. experimentalDecorators also matters. Record both per module.

## tsrs role
ROLE: an optional, offline, build-time type oracle. tsrs is NOT the engine parser and is NOT linked into the engine.

```
tsconfig ─► ep9-typecheck (separate binary; links tsrs) ─► side table per file:
              tsrs parser + full checker                    {hash of the decoded text,
              N checkers in parallel                         records sorted by (start,end,kind):
                                                             repr category, class-id = decl file:offset,
                                                             slot candidate, call-target decl, non-null,
                                                             enum const, dts/lib flag}
source ─► ep9 (oxc_parser + oxc_semantic) ─► join on (start,end)+kind ─► FactSnapshot(HINTED) ─► shared optimizer
```

EVIDENCE THAT IT WORKS:
- A working prototype (work/tsrs-type-oracle) drives Program + Checker through public API and path deps, with no tsrs changes.
- Join coverage to oxc 0.155 spans:
  - 99.94% of 556,724 nodes on TypeScript src/compiler;
  - 99.64% of 1.07M nodes on all of src/;
  - 0 kind mismatches;
  - CRLF and multi-byte UTF-8 align byte-exactly.
- Cost: 18 MB of TS checked and exported in 0.87 s wall with 16 checkers. The output is byte-identical to the single-checker run.
- Precision on typed class code: 0.2% any, 100% of member accesses resolved, 97% of calls with a resolved target.

WHY IT STAYS A SEPARATE BINARY:
1. tsrs pins unicode-id-start =1.1.2 (Unicode 15.1). Linking it with oxc drags oxc down too, and oxc then fails 16 test262 Unicode 16/17 identifier tests.
2. tsrs reserves a fixed 32 GiB at 0x4001_0000_0000, keeps arenas until process exit, has about 220 globals and abort paths.
3. The internals churn: about 200 commits a week, and the oracle had 6 compile errors against a 15-day-old tsrs. A side-table schema owned by ep9 absorbs this.
4. tsrs is Apache-2.0 and derived from TypeScript, so its license obligations stay inside a build tool.

SIDE-TABLE RULES:
- Hash the decoded text. tsrs strips a BOM, transcodes UTF-16 and repairs bad UTF-8, which shifts offsets; the BOM file joined at 0% until fixed.
- Key JSDoc casts on the ParenthesizedExpression span, because a JSDoc cast shares its span with the inner expression.
- Use a binary format of about 20 B per node (the JSON prototype is 195 B per node).
- Emit no type strings: they cost +620 MiB.

PARSER VERDICT: unchanged, and stronger.
- oxc_parser + oxc_semantic stays the only parser for code that runs (100% of test262 early errors, vs 26.8% for the tsrs parser).
- tsrs's parser runs only inside the oracle, to feed its checker. Its weak points (always-strict binder, JSX in .js, JSDoc AST rewrites) never reach execution.
- Dual mode adds one requirement: two parsers read the same bytes, so the join must check content hash plus (start,end,kind), and drop hints on a mismatch.
- tsrs is optional in every mode:
  - JIT mode needs no tsrs.
  - AOT for plain JS uses only PROVEN and PROFILED facts.
  - tsrs only adds HINTED facts for .ts projects.
- ep9 must still strip TS itself through oxc. When it does, it must apply the module's useDefineForClassFields and decorator semantics.

## Architecture
ONE ENGINE, THREE DEPLOYMENT MODES, ONE OPTIMIZER FED BY "FACT SNAPSHOTS"

```
 .js/.ts ──► oxc_parser + oxc_semantic ──► BytecodeGen (deterministic, hashed: bytecode is an ABI;
   │                                          the bytecode offset is the deopt/OSR identity)
   │                                                   │
   │ (build time, .ts only, optional)                  │
   └─► ep9-typecheck (tsrs, separate binary)           │
          └─► hint side table (span+kind, text hash)   │
                         │                             │
 ┌──────────────── FACT SOURCES (one schema = FactSnapshot = CacheIR-like IC feedback) ──────────────┐
 │ PROVEN   module graph, const, no-direct-eval scopes, own range analysis      -> no guard          │
 │ RUNTIME  FeedbackVector / IC data (JIT, hybrid)                              -> guard + deopt     │
 │ PROFILED training-run profile (AOT, optional; runtime feedback overrides it) -> guard             │
 │ HINTED   TS types from side table (unsound)                                  -> guard, never throw│
 └───────────────────────────────────────────┬───────────────────────────────────────────────────────┘
                                             ▼
 ┌──────────── RUNTIME (always linked) ─────────┐     ┌──── SHARED OPTIMIZER (S4 mid tier, later S5) ────┐
 │ Rust interpreter + data ICs                  │     │ SSA CFG from bytecode + FactSnapshot             │
 │ NaN-box values, ShapeId, elements kinds      │     │ facts: proven ⊑ predicted{source, confidence}    │
 │ GC: non-moving Immix, conservative stack     │     │ Guard(kind) -> policy per mode: Deopt | SlowPath  │
 │ snapshot: roots/builtins/atoms/class shapes  │     │ Assume(fuse) -> explicit check | dependency      │
 │ bytecode compiler (eval/new Function/lazy)   │     │ HeapConst -> RootRef / ShapeRef / UnitConst      │
 │ source text (Function.prototype.toString)    │     │ repr select, inlining, LICM, regalloc            │
 │ deoptimizer (eager + lazy via ret-addr swap) │     │ FP: no FMA contraction, exact ToInt32            │
 │ fuse registry (GuardFuse | InvalidatingFuse) │     └──────────┬────────────────────────┬──────────────┘
 │ dispatch slot per fn: interp|base|opt|AOT    │          JIT emit                 AOT emit (build time)
 │ pc -> code registry (JIT code | AOT unit)    │          (abs ptrs, W^X heap)     (no text relocs; root reg
 └──────────────────────────────────────────────┘                                   + unit-table reg)
                                                                                         ▼
                      ep9 UNIT = .o (static link, iOS) or mmapped blob (servers):
                      [text][link table: roots/builtins/ShapeIds/atoms/fuses][IC-site tables, RW]
                      [deopt translations + safepoints + pc->bytecode maps][bytecode][source]
                      [header: engine build hash, value-repr/GC-barrier version, CPU features] -> mismatch = use bytecode
```

MODES

| Mode | Tiers | Peak owner | Use |
|---|---|---|---|
| JIT (default) | interp -> baseline -> mid -> (top) | JIT | servers, CLI, smallest app payload (bytecode only) |
| AOT-jitless | interp + baseline-AOT for PGO-hot functions + optimized-AOT for the hottest | AOT | iOS, W^X-strict embedders |
| Hybrid | AOT units = tier-0 optimized code; feedback keeps flowing; JIT re-optimizes hot code (.NET R2R, Leyden JEP 544) | JIT | fast start + full peak |

GUARD-FAILURE POLICY (lives in the IR, not the backend)

| Fact | JIT | Hybrid | Jitless |
|---|---|---|---|
| proven | no guard | no guard | no guard |
| runtime-profiled | guard -> eager deopt + exit counter | same | n/a |
| PGO / TS-hinted | guard -> deopt | guard -> deopt -> re-profile -> JIT | guard -> inline slow path, or deopt within an exit budget, then pin the function to baseline-AOT (ART 'forever deopt' lesson) |
| global invariant | dependency -> lazy deopt + swap dispatch slot | same | explicit fuse check (load + compare) or dependency |

WHY THIS SHAPE
- Dart, Graal, RyuJIT, ART, Leyden and Hermes all use one optimizer with two fact sources.
- To compete, typed AOT needs exactly the mid-tier passes: int32/range inference, inline array access, shape guards, inlining. So the "deep optimizer" is the S4/S5 optimizer, run at build time.
- AOT still needs the interpreter, the bytecode compiler and the source text: eval, new Function, Function.prototype.toString. Every full-language AOT system (Hermes, Dart 2024, Graal Crema, Hopc, scriptc) keeps an interpreter for this.

SIZE NOTE
- JIT mode ships bytecode (6-19x smaller than native).
- AOT runtimes can drop the JIT tiers but not the bytecode compiler: it is about 2.4 MB of Static Hermes' 6.0 MB runtime.
- AOT must select a PGO hot set. Native code for all of the TypeScript workload would be about 2.5-4.4 MB plus metadata.

## Added hard decisions
- **D1 Bytecode is a serialized ABI**: Deterministic, hashed, versioned bytecode with constants referenced by index (no heap pointers) and serialized liveness tables. The bytecode offset is the only deopt/OSR identity. Each AOT unit ships its bytecode (inlined callees included), or the bytecode must regenerate bit-identically from the source.
  - Why: Bytecode is the deopt target for AOT and JIT code, the cache format and the first 'AOT' product. Graal Ristretto had to preserve AOT bytecodes explicitly before it could inline them.
  - Evidence: Hermes .hbc 14.9 ms vs 836 ms from source (startup-results.txt); V8 code cache stores only bytecode (code-serializer.cc:254-277); graal substratevm/docs/ristretto-bytecode-preservation.md
- **D2 One frame layout and calling convention for interpreter, baseline, opt and AOT code**: Use the S0 canonical frame for all four code kinds. Calls go through a per-function dispatch slot. A pc-to-code registry lets exceptions, GC, Error.stack, the profiler and the debugger walk AOT frames. Decide early: VM register stack (Hermes model) or native-stack generated interpreter (S1.5).
  - Why: Then AOT and JIT functions call each other through the normal call path, and the interpreter can be the landing pad for any deopt.
  - Evidence: Hermes SHStackFrame shared by interpreter, JIT and native (sh_stack_frame_layout.h:14-120; StaticH.cpp:413-466); SubstrateVM Deoptimizer.java: deopt target is always AOT code
- **D3 IR fact model with provenance**: Each fact is proven or predicted{source: runtime|PGO|TS-hint, confidence}. The guard-failure policy (Deopt | SlowPath | Throw-only-in-sound-mode) lives in the IR, not the backend. The IR holds no raw heap constants, only RootRef, ShapeRef and UnitConst nodes. The FactSnapshot schema equals the IC feedback schema, so TS hints, PGO and runtime feedback are interchangeable.
  - Why: The same optimizer serves JIT and AOT. Unsound TS facts must never be treated as proven.
  - Evidence: JSC DFGAbstractValue.h:55-103 (proven vs predicted); Warp CacheIR snapshot; Dart 276 is_aot branches in one compiler; 24/24 unsound TS programs
- **D4 Position-independent deopt metadata**: Per exit, store the pc offset and each live value's location (register, stack, constant index, or materialize). Use one shared deopt entry. Lazy deopt swaps return addresses and never patches code. Exit counters live in RW data.
  - Why: AOT text is read-only and may be code-signed (iOS). TurboFan-class code has a deopt point every 60-80 bytes, so this format sets unit size.
  - Evidence: v8-deoptimization.md:19-33; v8size/meta-summary.txt (12.5-17.1 deopt points per KB)
- **D5 Startup snapshot with stable indices and deterministic shapes**: Roots, builtins, atoms, fuses and class root shapes get stable indices. The object header's u32 shape field holds a ShapeId (an index), not a pointer. Property-add order, slack tracking and dictionary-mode heuristics are deterministic and versioned.
  - Why: AOT shape guards only hit if runtime objects end up at the same transition-tree nodes that the AOT code guards.
  - Evidence: dual-mode-architecture D5; V8 embedded builtins via root register (v8.dev/blog/embedded-builtins)
- **D6 Code unit format with zero text relocations**: Every external reference goes through a reserved root register or the unit's link table. The same container can be emitted as a .o (iOS static link) or an mmapped blob. The header carries the engine build hash, value-repr and GC-barrier version, and CPU features. On a mismatch, the engine falls back to bytecode.
  - Why: Isolate-independent code is required. Leyden rejects AOT code when CPU features or GC differ. iOS needs the code linked into the signed binary.
  - Evidence: wasmtime code_memory.rs:178-195 (refuses text relocations); JEP 544; V8 short-builtin-calls blog (4 GiB / 128 MiB call-range issue)
- **D7 IC state only in writable data**: Use CacheIR-like guard+action lists with stub fields stored as data. Handlers come from a shared precompiled stub corpus. No code patching anywhere.
  - Why: AOT code is immutable. Static Hermes and weval show that data ICs work inside AOT code.
  - Evidence: Static Hermes SHReadPropertyCacheEntry in emitted C (richards-untyped.c:186-190, sh_mirror.h); weval 2,367 precompiled stubs, 2.77x over interpreter
- **D8 Global-invariant registry with two kinds**: GuardFuse: an explicit load+compare check that is cheap in AOT. InvalidatingFuse: a registered dependency that triggers lazy deopt and a dispatch-slot swap. TS-derived class-hierarchy and enum-folding facts use fuses.
  - Why: JS can patch prototypes, Array species and Promise.then at any time, and AOT code cannot be recompiled on the spot.
  - Evidence: SpiderMonkey RealmFuses.h:16-40; V8 protectors.h (22 protectors); JSC Watchpoint.h
- **D9 The bytecode compiler, interpreter and source text are always linked**: No 'lean' runtime in full-ECMAScript mode. eval, new Function and Function.prototype.toString work in every mode.
  - Why: test262 requires them (1,791 files use eval or Function(; 80 test toString). Every full-language AOT system kept an interpreter.
  - Evidence: Hermes lean VM aborts ('Lean VM does not support bytecode generation'); compiler = 2.4 MB of 6.0 MB; Dart re-added an interpreter in 2024 (8fbca8ba67); Porffor eval only for strings known at compile time
- **D10 One Rust backend for JIT and AOT in v1**: The S2 MacroAssembler gets abstract relocation kinds and emits either JIT code or AOT units. Cranelift or LLVM only for an optional AOT top tier, with deopt exits written as calls.
  - Why: Then the deopt and frame formats exist only once. Cranelift has no deopt stackmaps.
  - Evidence: execution-tiers.md S2/S5; Cranelift 11-21 µs per bytecode
- **D11 Jitless guard policy: no deopt loops**: In jitless mode a failed hint guard takes an inline slow path, or deopts within an exit budget. After that the function is pinned to its baseline-AOT version.
  - Why: With no JIT, nothing re-profiles or recompiles, so a deopt can repeat forever.
  - Evidence: ART inliner.cc:614-636 UseOnlyPolymorphicInliningWithNoDeopt ('forever deoptimizations')
- **D12 Floating-point semantics in every backend**: No FMA contraction (-ffp-contract=off or equivalent), exact IEEE-754 and ToInt32. A differential test checks the nbody checksum against V8.
  - Why: A C/LLVM backend silently changes JS results.
  - Evidence: Porffor nbody -0.16919383316763523 vs V8 -0.169193895234998, fixed with -ffp-contract=off (aot-vs-jit/porfcheck; results-table.md)
- **D13 TS hint side table is an ep9-owned, versioned schema**: Offline ep9-typecheck output in binary form (about 20 B per node). It is keyed by (start, end, kind) plus a hash of the decoded text, and carries dts/lib flags plus the tsrs version and the ep9 IR version. Hints in files with type errors are dropped. The table can be embedded as a hint section in the bytecode cache.
  - Why: This isolates tsrs churn (6 compile errors over 15 days), BOM/UTF-16 offset shifts and the oxc Unicode pin.
  - Evidence: tsrs-type-oracle findings: join 99.94%, BOM 0% -> 100% after the fix, unicode-id-start =1.1.2, 195 B/node JSON
- **D14 TS-to-JS semantic flags are part of module identity**: Record useDefineForClassFields ([[Set]] or [[Define]]) and decorator mode per module, in the bytecode header and the hint table. Default to [[Define]], as node strip-types does.
  - Why: The same source gives different object shapes. 4 of 6 real codebases use [[Set]].
  - Evidence: ts-type-trust/fields/f.ts (own keys [] vs ["z"]); vscode tsconfig useDefineForClassFields:false

## Staged order
- A0 with S0 (specs): write D1-D14 next to the five S0 decisions (value repr, bytecode, frame, IC, object model). Fix ShapeId, the FactSnapshot schema (= CacheIR-like feedback schema) and the unit container. Set up differential testing (ep9 vs node/QuickJS) and the test262 runner.
- A1 with S1 (Rust interpreter + data ICs): add a serializable, mmap-able bytecode cache and a startup snapshot. This is the first 'AOT' product, with no machine code, and it gives the largest measured startup gain: Hermes .hbc 14.9 ms vs 836 ms; qjsc 37 vs 380 ms.
- T1 alongside S1-S3 (cheap, the prototype exists): turn the tsrs oracle into the ep9-typecheck tool with the binary side table. Measure hint precision (any share, class-receiver share, slot hits) on ep9's target apps, not only on libraries. Do not use hints for codegen yet.
- A2 with S2 (JIT infra): MacroAssembler with abstract relocation kinds, plus a unit writer/loader (object crate) and build-time embedding of IC stubs. This exercises the AOT path early, the way V8 embedded builtins do.
- A3 with S3 (baseline JIT): add an AOT emit mode, `ep9c --baseline`, for jitless targets. Expect 1.6-2.8x the interpreter (Hermes native 1.6-2.0x, weval 2.77x). Code is about 7x the bytecode, so compile only PGO-selected functions by default. Do this stage early only if iOS/jitless embedding is a near-term target.
- A4 with S4 (speculative mid tier, the minimum tier for speed): the IR consumes FactSnapshots from runtime feedback and from the AOT driver (PGO first, proven module facts second, TS hints third). Turn on hybrid mode: AOT units become tier-0 optimized code and the JIT re-optimizes. Now measure the TS-hint gain: fewer 'insufficient feedback' deopts, shorter warm-up on a 1-2 s tsc-like run. Kill or keep the hint path based on this number.
- S5 (optional top tier): Turboshaft-class passes or a Cranelift lowering. It is shared by JIT and optimized-AOT emit.
- A5 (optional, product decision): an opt-in sound typed module mode (nominal classes, exact fields, checked boundaries, int32 range inference, unboxed field layouts, unlimited inline slots, inline bounds-checked array access). This is the only measured path to beat V8 at peak, on numeric class code, by about 1.1-1.4x. It runs outside test262 conformance mode.

## Numbers
| Metric | AOT | JIT | Verdict | Source |
|---|---|---|---|---|
| nbody, ms per round (2.5M steps) | Static Hermes sound-typed native 173 | V8 full 90; V8 Maglev cap 246; JSC 112 | AOT 1.92x slower than V8; 1.42x faster than Maglev | work/aot-vs-jit/results-table.md (re-read) |
| nbody, ms (5M steps) | scriptc TS-typed, unboxed f64 structs: 157 | V8 196-223 (another run implies about 180) | AOT 1.15-1.4x FASTER (the only win) | work/aot-precedents/scriptc-results.txt (re-read) |
| nbody ceiling, ms/round | C -O2 -ffp-contract=off 74; AssemblyScript Wasm 81 (JSC) / 99 (V8) | V8 90 | at most about 1.22x faster with JS float rules | work/aot-vs-jit/bound.log |
| Richards, ms/round | Static Hermes typed 288 | V8 91; Maglev 133; JSC 74 | AOT 3.16x slower | results-table.md |
| Richards x4000, ms | scriptc 259; Static Hermes typed 585 | V8 182-192; Maglev 263-308 | best typed AOT ≈ Maglev; 1.4x slower than V8 | aot-precedents richards-results.txt |
| raytracer, ms/round | Static Hermes typed 148 | V8 96; Maglev 152 | 1.54x slower; ties Maglev | results-table.md |
| Octane, untyped, 7 kernels (fraction of V8 full) | Static Hermes native 0.094; Porffor 0.078 (6 kernels) | Hermes baseline JIT 0.117; Maglev cap 0.553; JSC 1.041 | untyped AOT ≈ baseline JIT | work/aot-vs-jit/octane-table.md |
| Real app: tsc, untyped (Meta, 2026-10-09) | Static Hermes native 1.20 s | node 354 ms | AOT 3.4x slower | speakerdeck Mikov 2026 |
| Untyped JS in a typed AOT compiler | scriptc Richards 20.6 s | V8 about 0.19 s | 107x slower (types-or-nothing cliff) | scriptc-results.txt |
| Hello-world startup | Porffor 1.47 ms; QuickJS 1.99; Hermes .hbc 4.03; Static Hermes native 4.87 | node 23.5; bun 5.77 | AOT/bytecode 5-16x faster than node | work/aot-vs-jit/startup.json |
| Load 9.1 MB of TS + 1 call | Hermes .hbc 14.9 ms; qjsc 37 ms | node cold 176 / compile cache 88; bun 165 / --bytecode 77.5 | serialized bytecode is the big win | aot-precedents/startup-results.txt |
| Peak RSS | Static Hermes 10-17 MB; native-image 57 MiB | node 50-62 MB; HotSpot C2 870 MiB | 3-15x less (Java heap defaults differ) | results.json; awfy_summary.txt |
| Native bytes / bytecode bytes | Static Hermes untyped 10-14x, typed 3.4-4.7x | Sparkplug 6.9x; Maglev 11.6-19x; TurboFan 11.9-28.6x | JIT payload is smaller | dual-mode-arch v8size/all.txt, sh/size-results.txt |
| Warm-up | native-image first iteration 0.9-1.4x steady | V8 kernels: 2-3 ms to peak (1.4-4 ms excess); tsc: 26-30% of a 1.6 s run; C2 2.07x | AOT wins only on short runs | warm.log; warmup-cli-tsproj.json; awfy |
| JIT compile CPU (tsc, 1.62 s wall) | 0 | 1.58 s background CPU | AOT saves CPU, not wall time | prof/trace-opt.txt |
| Dart AOT/JIT time (>1 = JIT faster) | poly 2-5 sites 1.94; Richards 1.19-1.28; provably mono 0.85; megamorphic 0.50 | - | polymorphism decides | dual-mode-vms/dartbench/dart_summary.txt (re-read) |
| .NET NativeAOT vs tiered JIT + PGO | Dictionary code 2.1-5.3x slower | - | PGO inlining and guarded devirt win | dotnet/runtime#129895 |
| Graal AWFY, time vs C2 | native-image no PGO 1.01-1.02; with PGO 0.89-0.92 | Graal JIT 0.92 | AOT ≈ JIT only with a training profile | awfy_summary.txt (re-read) |
| TS types on 6 real codebases | int-typed numeric exprs 1-9% (TS compiler 37%, enums); class receivers 0.4-61% | - | little layout or int info | ts-type-trust/results/summary.txt |
| V8 deopt mix (tsc check, 1,693 deopts) | - | wrong map 830 (49%); insufficient feedback 554 (33%); not a Smi 7 (0.4%) | TS types address 33% at most | ts-type-trust/tscheck/deopt.log (recounted) |
| Guard-removal ceiling at peak | - | 2.2-4.6% (IISWC'16); about 8% (IISWC'21 abstract); StrongScript -0.9..+22% | small | papers |
| V8 `x;` class fields (TS ES2022 emit) | - | V8 517 vs 90 ms (5.7x); JSC 112 vs 114 | V8-specific; NaN-boxing avoids it | results-table.md |
| tsrs oracle cost | 18 MB TS check+export 0.87 s wall (16 checkers); join 99.94% on 10 MB | - | fast enough for build time | tsrs-type-oracle out/ |

## Risks
- Overselling: the user's goal 'AOT faster than V8 from TS types' is not supported in general. The only measured win is scriptc nbody (1.15-1.4x), and it uses a subset that refuses eval, rejects some valid JS and terminates on typed-array out-of-bounds reads. If ep9 sets this as the main goal, the architecture will chase a 0-5% peak gain on ordinary TS.
- Effort: a V8-class top tier is multi-person-year work (Maglev about 4.4 MB of source; TurboFan + Turboshaft about 13.7 MB). Dual mode doubles the test matrix (mode × tier × platform). Dart keeps about 276 AOT branches inside one compiler. Meta's Static Hermes still has no speculative tier.
- Naive AOT can lose to the existing JIT. Hermes untyped native was slower than Hermes' own baseline JIT on every kernel (e.g. 242 vs 216 ms; 980 vs 920 ms).
- TS-hint value is unmeasured in any JS engine. Pre-filled feedback and guarded hints are hypotheses. With NaN-boxing, the V8 class-field gain mostly disappears (JSC 112 vs 114 ms). Put a kill criterion at A4.
- Deopt loops in jitless mode. A failed hint can deopt forever with no JIT to recompile. Exit budgets and slow-path variants are mandatory, and they add code size.
- Code-size blowup: native code is 6-19x the bytecode bytes, and units also carry bytecode, deopt maps, source and the snapshot delta. Whole-program AOT makes the app payload about 10x larger; Static Hermes tsc grew 26.3 to 69.8 MB. PGO hot-set selection is required.
- Shape determinism: AOT shape guards miss if runtime transition order, slack tracking or dictionary heuristics differ from the build. useDefineForClassFields changes shapes for the same source.
- Versioning: any change to value repr, object layout, GC barrier, frame or IC layout invalidates all AOT units. Units need a hash/CPU/GC check and a safe fallback to bytecode.
- Security: loading AOT units runs native code, so mmapped units need signature or hash checks. Speculative tiers are the main JIT exploit class. Fuzz interp vs baseline vs opt vs AOT differentially, and check TS hints especially, since they are wrong by design in edge cases.
- Training profiles can lock in worse code. In the Leyden Havlak run, a replayed profile gave a 31% faster first iteration but a 20-40% slower steady state. Runtime feedback must override profiles.
- tsrs dependency risk: about 200 commits a week, no published crates, and 'faithful port' policy. Linking tsrs with oxc pins Unicode 15.1, which breaks 16 test262 tests. Keep tsrs in a separate tool only.
- A sound typed mode would conflict with test262: throwing casts and bounds checks change JS semantics. It must be opt-in per module and excluded from conformance claims.
- Floating-point: an LLVM/C backend contracts to FMA and changes results unless that is disabled (Porffor nbody).
- Evidence limits: the machines were shared (load average 9-70), so single cells are ±10-20%. The workloads are mostly small monomorphic kernels (Richards, nbody, raytracer) that favor typed AOT. The typed sources are hand rewrites. No typed AOT has been measured on a large real app against V8.

## User decisions
- Q: What is the product claim for AOT?
  - Options: (a) AOT = startup, memory, predictability and jitless mode; the JIT owns peak. (b) (a) plus an opt-in sound typed module mode that targets beating V8 on numeric class code. (c) General 'AOT faster than V8' from TS types.
  - Recommendation: (a) now, (b) as optional stage A5. Drop (c): every measured system says no (Static Hermes typed 1.5-3.2x slower than V8; untyped AOT ≈ baseline JIT; the C ceiling on nbody is 1.22x).
- Q: Is iOS / jitless embedding a near-term target?
  - Options: Yes: build the baseline-AOT emit (A3) right after S3. No: only the bytecode cache + snapshot (A1) until S4.
  - Recommendation: Decide now, because it moves A3 by a year. If unsure, ship A1 first. It gives the largest startup gain (14.9 vs 836 ms) with no machine code.
- Q: Should ep9 offer a sound typed dialect, and with which semantics?
  - Options: (a) None. (b) Static Hermes style: Flow-like exact objects, nominal classes, throwing casts, no holes. (c) A TS-syntax nominal-class subset (STS/StrongScript style) checked by tsrs plus ep9 boundary checks.
  - Recommendation: Defer to A5. If built, choose (c), because users write TS, not Flow, and Static Hermes' -parse-ts path failed on real TS. Keep it per module, opt-in and out of test262 mode. Expect about 1.1-1.4x over V8 only on numeric, monomorphic class code.
- Q: How much should ep9 invest in TS-type hints before measuring?
  - Options: (a) Build the full hint pipeline into the optimizer early. (b) Build the cheap offline oracle and side table now, use hints only from S4, and gate on a measured gain. (c) Skip TS types.
  - Recommendation: (b). The oracle prototype already works (99.94% join, 0.87 s for 18 MB). The codegen gain is unmeasured and bounded at about 0-5% peak. The upside is warm-up and cold paths (33% of tsc deopts were 'insufficient feedback').
- Q: What does AOT compile: the whole program or a PGO hot set?
  - Options: (a) All functions (simple, payload about 10x larger). (b) A training-run hot set (ART speed-profile, Leyden). (c) Baseline-AOT for everything plus optimized-AOT for the hot set.
  - Recommendation: (b) by default and (c) for jitless. Native code is 6-19x the bytecode bytes. Runtime feedback must always override training profiles (Leyden Havlak regression).
- Q: Which TS class-field semantics does ep9 use when it strips types itself?
  - Options: (a) Always [[Define]], like node strip-types. (b) Follow tsconfig useDefineForClassFields and experimentalDecorators when a tsconfig exists.
  - Recommendation: (b) with (a) as the default. Record the choice per module in the bytecode header and the hint table. 4 of 6 real codebases compile with [[Set]], which gives different shapes.
- Q: Interpreter frame home, which the AOT emitter must match
  - Options: (a) Rust VM register stack + native control stack (Hermes model; works today). (b) Native-stack generated interpreter (S1.5), so deopt and OSR become plain frame copies.
  - Recommendation: Start with (a), and keep the canonical frame layout identical so S1.5 can switch later. The AOT emitter (A2/A3) is the natural tool to build and embed S1.5.
