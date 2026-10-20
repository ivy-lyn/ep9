# dual-mode-architecture

## Summary
The core idea can be built, but the measurements here change its framing. Every production system that ships both modes runs one optimizing compiler fed by different "fact sources": Dart runs one pass pipeline with an AOT flag, ART runs one optimizing compiler for JIT and dex2oat, Graal serves HotSpot JIT, native-image and Truffle, and HotSpot JEP 544 (Integrated, JDK 28) stores C1/C2 code from a training run. AOT code is used as the startup/no-JIT tier, and the JIT keeps re-optimizing on top of it (.NET R2R → Tier1Instrumented → Tier1; ART prefers JIT code; Leyden falls back to the interpreter and JIT).

What I measured on this M-series Mac:
- **Static Hermes** (static_h 5cf80e4, built here) is the only JS engine that ships AOT-native + JIT + interpreter in one runtime.
  - Untyped AOT native code runs at baseline-JIT speed, not faster. Richards: 242 ms native vs 216 ms Hermes JIT vs 342 ms V8 Sparkplug.
  - Sound-typed AOT (the source rewritten into Hermes' sound typed dialect) is 4-6x faster than untyped (Richards 60 ms, nbody 27 ms). It still trails V8 peak: 18 ms on both, Maglev 29/43 ms, JSC 23/21 ms.
- **AOT without speculation lands near baseline tier.** Other data points: SpiderMonkey weval AOT is 2.77x its interpreter, and Hermes native is 1.6-2.0x its interpreter. Reaching mid-tier speed needs speculation, from sound types or profiles, plus guards and deopt or slow paths. TypeScript types are unsound by design ("Non-goal: apply a sound type system"), so in a full-ECMAScript engine they can only be hints that sit behind guards.
- **Native code costs 6-19x the bytecode bytes.**
  - V8 13.6 arm64: Sparkplug 6.94x (1,728 functions, 452 KB bytecode); Maglev 11.6x and TurboFan 11.9x without inlining, 19-29x with inlining.
  - Hermes: untyped native 10-14x, typed 3.4-4.7x.
  - .NET R2R assemblies grow 2-3x.
  - So "JIT for smaller size" is right: JIT ships bytecode and compiles only what runs (V8 lazy compile produced 246 KB of bytecode vs 364 KB eager on the TypeScript workload).
- **AOT mode still needs the interpreter, the bytecode compiler (for eval / new Function, used in 1,791 test262 files) and the source text (Function.prototype.toString).**
  - Hermes' compiler is 2.4 MB of a 6.0 MB static runtime (lean VM: 3.6 MB).
  - Dart re-added a bytecode interpreter to its AOT runtime in 2024 for dynamic modules, after removing bytecode mode in 2020.
  - GraalVM native-image is adding the Crema interpreter plus the Ristretto JIT, which deopts back to the interpreter.
  - Porffor (100% AOT) supports eval only for compile-time-known strings.

**What dual mode adds to the JIT-only plan:**
- Bytecode becomes a serialized ABI and the universal deopt target.
- One frame layout and calling convention for all four code kinds.
- An IR that separates proven facts from predicted ones and has a per-mode guard-failure policy.
- Position-independent deopt and safepoint metadata.
- Lazy deopt by rewriting return addresses, never by patching code.
- A startup snapshot with stable indices: roots, builtins, ShapeIds, atoms, class root shapes.
- A code format with no text relocations (Wasmtime enforces this), where every heap or runtime reference goes through a reserved root register or the unit's link table.
- IC state only in writable data, never in code.
- A registry of global invariants with two kinds: explicit-check fuses and invalidating dependencies.
- Source retention.
- Artifact version, CPU-feature and GC-barrier checks.

## Recommendation
BUILD ONE ENGINE, THREE DEPLOYMENT MODES, ONE OPTIMIZER FED BY "FACT SNAPSHOTS".

The modes:
- **JIT mode** (default, smallest app payload): interpreter → baseline → mid tier → top tier, as in execution-tiers.md.
- **AOT-jitless mode** (iOS / W^X-strict): interpreter + AOT units. Every function gets baseline-quality AOT code; PGO-hot functions get optimized AOT code; deopt goes to the interpreter.
- **Hybrid mode** (servers / CLI startup): AOT units act as tier-0 optimized code (R2R / Leyden style). They keep writing feedback, and the JIT re-optimizes hot code.

```
 .js/.ts ──► oxc_parser+semantic ──► scope/alloc ──► BytecodeGen (deterministic, versioned; bytecode = deopt/OSR identity)
                                                        │
        ┌───────────────────────────────────────────────┴──────────────────────────┐
        ▼  (all modes)                                                             ▼  (ep9c AOT driver, build time)
 ┌──────────── RUNTIME (always linked) ────────────┐        ┌──────── FACT PRODUCERS ─────────────────────────┐
 │ Rust interp + data ICs (FeedbackVector,         │        │ PROVEN : module graph (ESM bindings, const,     │
 │   CacheIR-like stub data, shared stub corpus)   │        │          no-direct-eval scopes, spec invariants)│
 │ object model: ShapeId, NaN-box, elements kinds  │        │ PROFILED: training-run profiles (Jump-Start/ART)│
 │ GC (non-moving Immix, conservative native stack)│        │ HINTED : tsrs checker types (unsound → guarded) │
 │ startup snapshot: roots/builtins/atoms/shapes   │        └───────────────┬─────────────────────────────────┘
 │ code-range registry: pc → {JIT code | AOT unit} │                        │ FactSnapshot (same schema as runtime
 │ deoptimizer: eager exits + lazy (ret-addr swap) │   runtime feedback      │  feedback: CacheIR seqs per IC site,
 │ fuse/protector registry (Guard | Invalidating)  │──────────────┐          │  call targets, value types, fuse ids)
 │ dispatch table: fn → entry (interp|base|opt|AOT)│              ▼          ▼
 │ bytecode compiler (eval/new Function/lazy)      │     ┌──────── SHARED OPTIMIZER (one crate) ────────────┐
 └─────────────────────────────────────────────────┘     │ graph builder(bytecode, FactSnapshot) → SSA CFG  │
                                                         │ value facts: proven ⊑ predicted{src,conf}        │
                                                         │ Guard(kind) → policy: Deopt | SlowPath | Throw   │
                                                         │ Assume(fuse) → explicit check | dependency       │
                                                         │ HeapConst → RootRef/ShapeRef/UnitConst nodes     │
                                                         │ repr selection, inlining, LICM, regalloc         │
                                                         └───────────┬──────────────────────┬───────────────┘
                                                              JIT emit (abs ptrs,      AOT emit (no text relocs,
                                                              W^X code heap)           root reg + unit-table reg)
                                                                                           ▼
                                        ep9 UNIT = .o (static link, iOS) or mmapped ELF/Mach-O blob (servers):
                                        [.text][link table: roots/builtins/ShapeIds/atoms/fuses][IC-site tables (RW)]
                                        [deopt translations + safepoints + pc→bytecode maps][bytecode][source]
                                        [header: engine build hash, value-repr/GC-barrier ver, CPU features]
```

**Guard policy** (lesson from ART: no deopt loops without a JIT):

| Fact | JIT mode | Hybrid | Jitless |
|---|---|---|---|
| proven | no guard | no guard | no guard |
| runtime-profiled | guard → eager deopt + exit counter | (same) | n/a |
| PGO / TS-hinted | guard → deopt | guard → deopt → re-profile → JIT | guard → inline slow path, or deopt with an exit budget, then pin the function to its baseline-AOT version |
| invariant | dependency → lazy deopt + swap dispatch slot | (same) | (same) |

**HARD-TO-CHANGE DECISIONS ADDED BY DUAL MODE** (on top of execution-tiers' value repr, bytecode, frame, IC and object model):
- **D1 Bytecode is an ABI.**
  - Generation is deterministic and the format is hashed.
  - Offsets are the only deopt/OSR identity.
  - Constants are referenced by index; there are no heap pointers.
  - Liveness tables are serialized.
  - It ships in every unit (inlined callees included: the Graal Ristretto lesson), or it must regenerate bit-identically from the source.
- **D2 One frame layout and calling convention** for interpreter, baseline, opt and AOT.
  - Calls go through a per-function dispatch slot.
  - argc is passed and missing args are padded with undefined.
  - A pc→unit registry lets exceptions, GC, Error.stack, the profiler and the debugger walk AOT frames.
  - Decide early between a Rust VM register stack plus native control stack (the Hermes model; works today) and a fully native-stack layout (needs the generated interpreter, S1.5). The AOT emitter is the natural way to build and embed S1.5.
- **D3 IR fact model.**
  - Facts are proven or predicted, with provenance.
  - The per-mode guard-failure policy lives in the IR, not in the backend.
  - No raw heap constants in the IR.
- **D4 Deopt metadata.**
  - Position-independent: pc offsets and value locations given as reg / stack / const-index / materialize.
  - One shared deopt entry.
  - Lazy deopt by return-address swap, never code patching.
  - Exit counters in RW data.
- **D5 Snapshot.**
  - Stable indices for roots, builtins, atoms and fuses.
  - ShapeId indirection; the gc-design header's u32 shape field must be an id or table index.
  - Class root shapes are created in the snapshot, and the transition tree is deterministic so runtime shapes converge onto the AOT-guarded shapes.
- **D6 Code format.**
  - Zero text relocations; all external references go through reserved registers or the unit table.
  - One container that can be emitted as a .o or as an mmapped blob.
  - A version, CPU and GC check that falls back to the bytecode on mismatch.
- **D7 IC layout.** IC state lives only in writable side tables. Handlers come from a shared precompiled stub corpus keyed by CacheIR shape, with stub fields stored as data.
- **D8 Global-invariant registry** with explicit-check fuses (AOT-cheap: load + compare) and invalidating dependencies.
- **D9 Source retention** for Function.prototype.toString and lazy compilation.
- **D10 Same Rust backend for JIT and AOT** in v1, so the deopt and frame formats exist once. Cranelift or LLVM only for an optional AOT top tier, with deopt exits written as calls.

**STAGED ORDER** (aligned with S0-S5 in execution-tiers.md):
- **A0 (with S0):** write D1-D10; fix ShapeId, the FactSnapshot schema (= CacheIR/Warp-snapshot schema) and the unit container.
- **A1 (with S1):** interpreter + data ICs + startup snapshot + bytecode cache. This is the first "AOT" product, with no machine code: V8 does exactly this.
- **A2 (with S2):** MacroAssembler with abstract relocation kinds + unit writer/loader (object crate) + build-time embedding of IC stubs and, optionally, the generated interpreter (V8 embedded-builtins analog). This exercises the AOT path early.
- **A3 (with S3):** the baseline compiler gets an AOT emit mode → `ep9c --baseline` for jitless targets.
  - Expect 1.6-2.8x the interpreter: Hermes native 1.6-2.0x, weval 2.77x, Sparkplug 1.6x.
  - Code costs about 7x the bytecode, so default to PGO-selected functions only.
- **A4 (with S4):** the mid-tier IR consumes FactSnapshots from runtime feedback and from the AOT driver (PGO first, TS hints second, module analysis for proven facts). Optimized AOT code uses guards plus deopt to the interpreter. Hybrid tiering keeps feedback flowing so the JIT can re-optimize (.NET/Leyden).
- **A5 (optional, product decision):** a sound "strict typed" dialect (Static Hermes route: about 4-6x over untyped AOT, but it changes semantics) and/or an LLVM/Cranelift AOT top tier for hot code.

**Do not promise "AOT faster than JIT"** for general JS:
- Measured: typed AOT reaches 33-67% of V8 peak (Richards 60 vs 18 ms; nbody 27 vs 18 ms).
- Untyped AOT ≈ baseline JIT.
- What AOT + types + PGO does deliver: instant warm start, jitless-platform speed and predictable latency. The JIT still owns peak.

## Findings
- [high] Production dual-mode systems share ONE optimizing compiler and switch speculation per mode. Dart: CompilerPass::RunPipeline(PipelineMode mode) runs one pass list; only a few passes are AOT-only (INVOKE_PASS_AOT: ApplyClassIds, TypePropagation, ApplyICData, OptimizeTypedDataAccesses, DelayAllocations), the call specializers differ (aot_call_specializer.cc 1,320 lines vs jit_call_specializer.cc 217), and there are about 276 is_aot()/FLAG_precompiled_mode branches across about 237k lines of compiler .cc. ART: 'JIT and AOT use the same compiler with a similar set of optimizations'. Graal: one compiler for the HotSpot JIT, native-image AOT and Truffle partial evaluation.
  EVIDENCE: dart-sdk 847b3b0 runtime/vm/compiler/compiler_pass.cc:251-355; aot/ and jit/ line counts (wc); grep count over runtime/vm/compiler/**/*.cc; https://source.android.com/docs/core/runtime/jit-compiler; Dart doc runtime/docs/compiler/optimization_levels.md ('conceptually only supports level O0 (the un-optimized code that is used as our deopt fallback) and level O2 ... both JIT and AOT strike more or less the same balance')
- [high] Dart AOT forbids speculation outright. Requesting a deopt label in precompiled mode is FATAL, devirtualization happens only when TFA proves the target, and AOT uses PC-relative calls within a loading unit plus a global dispatch table. Dynamic calls go through 'switchable calls' whose state (unlinked → monomorphic → single-target class-id range → linear cache → megamorphic dictionary) lives in data, not in code.
  EVIDENCE: dart-sdk runtime/vm/compiler/backend/flow_graph_compiler.cc:1243-1248 (FATAL 'Speculative instructions are not allowed in AOT'), :3503-3519 (CanPcRelativeCall only in precompiled mode); aot/dispatch_table_generator.cc; pkg/vm/lib/transformations/type_flow (15,606 lines of TFA); https://mrale.ph/dartvm/ (switchable calls, 'must not rely on any speculative assumptions')
- [high] ART's AOT compiler uses profile-recorded inline caches but deliberately emits NO deopt in AOT code. It does polymorphic inlining with a generic fallback instead, because AOT deopts can loop forever (the profile is not refreshed if the method never gets hot again) and a process that cannot JIT would stay interpreted. CHA devirtualization is JIT-only. When both JIT and AOT code exist, the JIT code is preferred.
  EVIDENCE: AOSP art main compiler/optimizing/inliner.cc:614-636 (HInliner::UseOnlyPolymorphicInliningWithNoDeopt), :369-371 ('No CHA-based devirtulization for AOT compiler (yet)'), downloaded to /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/dl/art-inliner.cc; https://source.android.com/docs/core/runtime/jit-compiler
- [high] GraalVM native-image: 'usually all image compiled methods cannot deoptimize'. Runtime-compiled (Truffle) code deopts into AOT 'deopt target' variants compiled with optimizations removed (PartialEscape, ReadElimination and FloatingRead are dropped, and constants are never spilled), so their frames can be rebuilt. A new Crema interpreter plus the Ristretto JIT deopts back to the interpreter. To inline AOT methods into JIT code, their bytecodes must be preserved explicitly (-H:JITPreserveBytecodesForRuntime), and 'this option does not enable re-JIT of arbitrary AOT frames'.
  EVIDENCE: oracle/graal 5820e1e substratevm .../hosted/code/DeoptimizationUtils.java:125,214,220-245; .../interpreter/ristretto/compile/RistrettoDeoptimizationSupport.java ('Ristretto compilations that deoptimize back to the Crema interpreter'); substratevm/docs/ristretto-bytecode-preservation.md; ReservedRegisters.java:50-89 (thread, heap-base and code-base registers)
- [high] HotSpot JEP 544 'Ahead-of-Time Code Compilation' (Integrated, JDK 28) stores C1/C2 code from a training run. 'AOT code and JIT code can coexist and are completely interoperable.' On deopt, or when AOT code is missing or incompatible, execution falls back to the interpreter and JIT. AOT code is rejected if the CPU features or GC (barriers) differ. Startup drops 65-80% with AOT code (vs 50-70% for the cache alone), and javac's first iteration improves by about 75% in total. Its predecessor jaotc (JEP 295) was removed in JDK 17 (JEP 410): 'little use', 'effort required to maintain it is significant'.
  EVIDENCE: https://openjdk.org/jeps/544 ; https://openjdk.org/jeps/410
- [high] .NET treats AOT (ReadyToRun) code as Tier0: 'optimized, not instrumented, no patchpoints'. If an R2R method is called more than 30 times it is re-jitted as Tier1Instrumented, then as Tier1 with PGO. R2R code reaches outside its 'version bubble' through indirection cells and fixups, and cross-bubble inlining is banned. R2R files are 2-3x larger (IL + native), and 'Ahead-of-time generated code is not as highly optimized as code produced by the JIT'.
  EVIDENCE: dotnet/runtime docs/design/features/DynamicPgo-InstrumentedTiers.md (mermaid flow); docs/design/features/tiered-compilation.md:55-71; docs/design/coreclr/botr/readytorun-overview.md:97-110,225-237; https://learn.microsoft.com/en-us/dotnet/core/deploying/ready-to-run
- [medium] Measured: AOT without speculation lands at baseline-JIT speed. Richards (Hermes harness, 400 iterations, in-process ms, best of 5): V8 full 18, JSC 23, V8 Maglev 29, Static Hermes typed AOT native 60, SH typed JIT 167, SH untyped JIT 216, SH untyped AOT native 242, SH typed interpreter 315, V8 Sparkplug 342, SH untyped interpreter 484, V8 Ignition 556, V8 jitless 566, QuickJS 802. Relative to QuickJS: V8 44.6x, Maglev 27.7x, SH typed native 13.4x, SH untyped native 3.3x, Sparkplug 2.3x.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/run_richards.sh → sh/richards-results.txt; build via scripts/build_sh.sh (shermes -O --static-link; hermesc -O); node 24.22, bun 1.4.3, QuickJS 2026-06-04; load avg about 11 during the run
- [medium] Measured: sound static types matter mostly for native AOT, but even sound-typed AOT trails V8 peak. nbody 400k steps, in-process ms, best of 5: V8 full 18, JSC 21, SH typed AOT native 27, V8 Maglev 43, SH typed JIT 60, SH untyped JIT 149, SH untyped native 169, SH typed interpreter 265, SH untyped interpreter 270, Sparkplug 514, QuickJS 791, V8 jitless 854. (An earlier hyperfine wall-time run showed typed native 31 ms 'beating' node at 50 ms only because of node's roughly 30 ms process startup.)
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/run_nbody_timed.sh → sh/nbody-timed-results.txt; sh/nbody-hyperfine.md; node empty-script startup 30-36 ms (hyperfine). Load avg about 50-67 during the timed run, so ±15%.
- [high] Static Hermes' speedups need a sound typed DIALECT, not TypeScript semantics. Its typed mode uses exact objects with fixed property order, nominal classes, sound arrays that are incompatible with untyped JS arrays, and checked casts that THROW TypeError on mismatch. richards-typed.js is a rewrite into ES classes. Its -typed -parse-ts path failed to compile nbody.ts at 5cf80e4 (TypedLib:36 parse error). TypeScript's own design goals list 'Apply a sound or provably correct type system' and 'emit different code based on the results of the type system' as non-goals. So for full-ECMAScript ep9, TS types are guarded hints, not proofs.
  EVIDENCE: hermes doc/TypedLanguage.md (Objects/Arrays/Classes/Runtime Typechecking sections); benchmarks/octane/richards-typed/CLAUDE.md, benchmarks.md (Meta's own numbers: typed native 65 ms vs untyped native 295 ms vs node 19 ms); local shermes -typed -parse-ts error; https://github.com/microsoft/TypeScript/wiki/TypeScript-Design-Goals
- [high] Even sound static types do not remove the value of runtime speculation. V8 added feedback-driven call_indirect inlining plus deopts to WebAssembly (statically typed, AOT-friendly) in Chrome M137: 1.59x average on Dart microbenchmarks and 1-8% on large apps (Flute +8%). HHVM's JIT overtook the HPHPc AOT compiler (+10% after the 2012 push), and shipping profiles rather than code (Jump-Start) cut warmup overhead 54.9% and improved steady state 5.4%. Hopc (an AOT JS compiler that uses guards and ICs) loses to JITs on standard suites and is competitive on 18 new benchmarks, with less memory.
  EVIDENCE: https://v8.dev/blog/wasm-speculative-optimizations (2025-06-24); https://hhvm.com/blog/2027/faster-and-cheaper-the-evolution-of-the-hhvm-jit (2013-12-11); https://engineering.fb.com/2021/03/03/developer-tools/hhvm-jump-start/ ; ICFP'21 'Of JavaScript AOT Compilation Performance' abstract (https://icfp21.sigplan.org/details/icfp-2021-papers/9/Of-JavaScript-AOT-Compilation-Performance)
- [medium] Native code costs 6-19x the bytecode bytes (answers Q6 / 'JIT for smaller size'). V8 13.6 arm64 (--log-code, per-SFI matching, OSR included): Sparkplug 6.94x over 1,728 functions / 452 KB of bytecode (per-benchmark aggregate 5.7-9.1x); Maglev 11.6x without inlining, 19x with; TurboFan 11.9x without inlining, 10.8-28.6x with. Static Hermes C-backend __text per bytecode instruction byte: untyped 10.0-14.2x, typed 3.4-4.7x. Code-size cost is large, but sound types make code denser.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/v8sizes.py, v8agg.py → v8size/all.txt, v8size/agg.txt; sh/size-results.txt (size -m *.o vs hbcdump function-info Size sum)
- [medium] A JIT compiles only what runs, which is the real size advantage. On the Octane TypeScript workload (2.5 MB source), V8 produced 246 KB of bytecode (1,010 functions) lazily vs 364 KB (2,274 functions) with --no-lazy. Of the lazily compiled code, 601 functions were Sparkplug-compiled (0.96 MB), 441 Maglev-compiled (1.17 MB) and 180 TurboFan-compiled (0.49 MB). An AOT that compiles everything at baseline density would emit about 2.5 MB, or about 4.4 MB at optimized density, before deopt and safepoint metadata. A PGO-selected hot set (ART 'speed-profile', Leyden training run) is the size-sane AOT design.
  EVIDENCE: v8size/typescript.default.log, typescript.eager.log, all.txt (typescript.default rows)
- [high] Fixed runtime cost that every mode carries: V8 13.6 embedded builtins total 1,873 builtins = 1.57 MB of machine code plus 511 bytecode handlers = 344 KB (arm64). A Static Hermes static executable is 6.0 MB, and linking the lean VM (no source compiler) gives 3.6 MB, so supporting eval / new Function costs about 2.4 MB. The lean binary aborts at startup because its console bindings need the JS compiler ('Lean VM does not support bytecode generation').
  EVIDENCE: v8size/richards.default.log (code-creation Builtin/BytecodeHandler sums); scripts/build_lean.sh output; ls of sh/richards-typed-native vs richards-typed-native-lean
- [high] An AOT-capable runtime must keep an interpreter and the source compiler. Dart removed bytecode mode (7588ed86de, 2020-10-28) and re-added an interpreter in 2024 for 'dynamic modules in the VM/AOT' (8fbca8ba67, 2024-08-15). Graal added Crema/Ristretto. .NET iOS mixes AOT with the Mono interpreter because Reflection.Emit is unavailable under AOT (and the interpreter is incompatible with NativeAOT). Porffor (100% AOT, 92.05% of 53,404 test262 tests on 2026-10-19) compiles eval and new Function only for compile-time-known strings. 1,791 test262 files use eval/Function(, and 80 test Function.prototype.toString. Hermes returns placeholders for toString because of AOT bytecode.
  EVIDENCE: gh api dart-lang/sdk commits?path=runtime/vm/interpreter.h; runtime/vm/interpreter.h (#if defined(DART_DYNAMIC_MODULES)); https://learn.microsoft.com/en-us/dotnet/maui/macios/interpreter ; repos/CanadaHonk_porffor compiler/codegen.js:2041-2075, test262/history.json; find+grep over repos/test262/test; hermes doc/Features.md:83
- [high] Static Hermes shows how one runtime runs interpreter, JIT and AOT-native frames together. One register-stack frame layout (SHStackFrameLayout: previousFrame, savedIP, savedCodeBlock, SHLocals, argCount, newTarget, callee, this, args) serves all three. AOT C code roots its temporaries through an SHLocals linked list, so its stack is precise without stack maps. Calls from native code dispatch on the callee kind: native fn pointer, JIT code pointer, or a nested _interpret re-entry. There is no speculation or deopt anywhere in Hermes; its JIT is a baseline JIT over bytecode.
  EVIDENCE: hermes include/hermes/VMLayouts/sh_stack_frame_layout.h:14-120; lib/VM/StaticH.cpp:413-466 (doCall, _sh_ljs_call); generated sh/richards-typed.c:790-830 (_sh_enter, locals.head); grep -ri deopt lib/VM/JIT returns nothing (Inlining.cpp 'deopt' means a regalloc quality drop)
- [high] IC data in AOT code works when ICs are data. Static Hermes AOT C keeps per-unit arrays (SHReadPropertyCacheEntry{clazz, slot}: 282 read and 128 write entries for Richards). It inlines 'if (obj->clazz == cache->clazz) load slot' and calls the runtime on a miss; the struct is a C mirror of the VM's PropertyCacheEntry (sh_mirror.h), so interpreter and AOT share the IC layout. SpiderMonkey's weval AOT dispatches each IC site through a stub pointer into a precompiled corpus of 2,367 CacheIR stubs, and CI fails when a new stub appears; it reaches 2.77x its interpreter on Octane (geomean 821 → 2273). Dart's switchable calls are the same idea.
  EVIDENCE: sh/richards-untyped.c:186-190, 2369-2384; hermes include/hermes/VM/static_h.h:744-780; include/hermes/VM/sh_mirror.h:1-40; https://cfallin.org/blog/2024/08/27/aot-js/
- [medium] JIT code embeds heap pointers that an AOT unit cannot contain. V8 TurboFan code carries 'full embedded object' relocations: 3.0-6.5 embedded heap objects per KB of code (e.g. Maps used in shape guards), 12.5-17.1 deopt points per KB (about one every 60-80 bytes), and RelocInfo equal to 33-79% of instruction bytes (this includes deopt-comment entries). An AOT format has to turn these into root/unit-table loads, and needs serialized deopt translations for each of those points.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/v8meta.py → v8size/meta-summary.txt (node --print-opt-code on richards, deltablue, raytrace, earley-boyer, typescript; TurboFan only); v8size/printopt.txt shows 'full embedded object <Map[28]>' + 'deopt reason (wrong map)'
- [high] Isolate-independent, position-independent code is a solved pattern. V8 embedded builtins load roots through a dedicated root register, use a builtins constants table, make pc-relative builtin-to-builtin calls, and are emitted by a build step into embedded.cc / .text (19% median V8 heap saving; the regression shrank from more than 5% to about 0.5% on Speedometer2). Wasmtime refuses to load precompiled .cwasm artifacts that contain any relocation outside .debug_* ('all builtins use the same trampoline mechanism'). SubstrateVM reserves thread, heap-base and code-base registers. The V8 code cache never stores machine code (CHECK !IsInstructionStream), only bytecode, and re-enqueues Sparkplug batch compilation after loading.
  EVIDENCE: https://v8.dev/blog/embedded-builtins ; bytecodealliance/wasmtime fa20b4c crates/wasmtime/src/runtime/code_memory.rs:178-195; graal ReservedRegisters.java:50-89; v8/v8 main src/snapshot/code-serializer.cc:254-277, 547-561
- [high] Lazy deopt and invalidation can be done without code patching, which is required for AOT text in read-only signed segments. V8 marks dependent code (set_marked_for_deoptimization) and patches the RETURN ADDRESS on the stack to a lazy-deopt builtin. V8 has 22 one-way isolate protectors (ArraySpeciesLookupChain, NoElements, PromiseThenLookupChain, ...). JSC WatchpointSet goes ClearWatchpoint → IsWatched → IsInvalidated. SpiderMonkey splits fuses into GuardFuse (checked explicitly) and InvalidatingFuse (registered dependency): exactly the two lowerings an AOT unit needs.
  EVIDENCE: v8/v8 docs/runtime/deoptimization.md:19-33 (fetched as work/dual-mode-arch/v8-deoptimization.md); src/execution/protectors.h (22 V(...) entries, v8 main ea84d23); WebKit Source/JavaScriptCore/bytecode/Watchpoint.h:154-156; mozilla-firefox js/src/vm/RealmFuses.h:16-40
- [high] The IR type lattice needs 'proved' and 'predicted' facts kept apart. JSC DFG keeps the AbstractValue (proved: SpeculatedType m_type, ArrayModes, StructureAbstractValue, constant) separate from per-node predictions. Hermes is moving its 16-bit IR type mask to an 'IR Type System v2' that carries nominal classes and arrays and generates runtime type descriptors for checked casts and guards. SpiderMonkey Warp shows the interface: feedback is a CacheIR snapshot taken on the main thread and transpiled into MIR. A static/TS fact producer can emit that same schema.
  EVIDENCE: WebKit Source/JavaScriptCore/dfg/DFGAbstractValue.h:55-103; hermes doc/plans/ir-type/ir-type-system-v2-design.md:1-40; https://hacks.mozilla.org/2020/11/warp-improved-js-performance-in-firefox-83/ (cited in research/execution-tiers.md)
- [medium] Full AOT once failed on Android: on Android 6 large apps took minutes to optimize at install, and system updates were slow. Android 7 switched to interpreter + JIT + profile-guided background AOT, with installs taking seconds. Profiles are dumped by the JIT and consumed by dex2oat.
  EVIDENCE: https://developer.android.com/about/versions/nougat/android-7.0 ; https://source.android.com/docs/core/runtime/jit-compiler
- [medium] Jitless targets need AOT linked into the signed binary. Third-party iOS apps cannot JIT (only EU/Japan browser-engine entitlements via BrowserEngineKit), and iOS 26 beta re-blocked even debug JIT for Flutter. So the iOS AOT format must be a static object linked into the app (Static Hermes, V8 embedded blob, Dart AOT), not a runtime-mmapped code file. Servers can use the mmapped-blob form (Wasmtime .cwasm, ART .oat, Leyden cache).
  EVIDENCE: https://developer.apple.com/support/alternative-browser-engines/ (via search); juejin.cn report on iOS 26 beta (secondary); research/execution-tiers.md MAP_JIT findings

## Risks
- Speculative AOT on jitless targets can loop through deopts forever, because nothing re-profiles or recompiles (ART avoided this by emitting no deopt in AOT). It needs exit budgets plus slow-path variants, and those grow code size.
- Code-size blowup: native code is 6-19x the bytecode, and AOT units must also carry bytecode, deopt and safepoint metadata, source and the snapshot delta. AOT of all functions would make the user's app payload roughly 10x larger than JIT mode. PGO selection is mandatory, not optional.
- TS types are unsound, so using them as facts without guards produces type-confusion bugs. TurboFan-class code has about one deopt point per 60-80 bytes; the guard density of TS-hinted AOT code may erase much of the gain. Not measured.
- Shape determinism: AOT guards against snapshot ShapeIds only hit if runtime object construction converges on the same transition-tree nodes. Property-add order, slack tracking and dictionary-mode heuristics must all be deterministic and versioned.
- Versioning: every change to value representation, object layout, GC barrier, frame or IC layout invalidates all AOT units (Leyden checks CPU features and GC). Units must be rebuilt per engine release and rejected safely on mismatch.
- Security: loading an AOT unit means executing native code. mmapped units need signature or hash checks, and AOT code paths need the same tier-differential fuzzing as the JIT (interp vs baseline vs opt vs AOT).
- Engineering cost: every IR change gets two consumers and a test matrix (mode × tier × platform). Dart keeps about 276 AOT branches inside one compiler. Static Hermes, Meta's AOT+JIT engine, still has no speculative tier after years of work.
- The performance numbers come from a heavily shared machine (load average 11-67). Richards and nbody are small kernels; typed variants are hand-rewritten sources, so typed-vs-untyped is not an apples-to-apples semantic comparison.
- Hermes' untyped AOT native code was slower than its own baseline JIT (242 vs 216 ms). A naive AOT (generic runtime calls through C) can lose to the JIT tier that already exists.

## Open questions
- How well do tsrs-checker types predict runtime shapes and call targets on real TS code (e.g., the share of property-access sites whose static receiver type is a class/interface with a stable layout, compared with runtime IC monomorphism)? This decides whether 'HINTED' facts are worth it beyond PGO; needs a joint experiment with the tsrs-checker workstream.
- How much of a Node/Deno app's hot code does a training run cover (the PGO hot-set size)? This sets the AOT payload size in hybrid and jitless modes.
- Should ep9 offer a sound typed dialect (Static Hermes-style exact objects and nominal classes, throwing checked casts)? It is a product/semantics decision: about 4-6x over untyped AOT, but it is not TypeScript semantics.
- Frame decision (also open in execution-tiers.md): keep the Rust interpreter's VM register stack and give JIT/AOT code a dual-stack frame (Hermes model), or move to a generated native-stack interpreter (S1.5) so deopt and OSR are plain frame copies?
- Deopt translation byte size per deopt point for a V8/JSC-class tier was not measured: V8 --print-opt-code does not print translation array sizes. This affects AOT unit size.
- Will iOS App Store size and launch-time limits allow statically linked AOT units of several MB plus the 2-4 MB runtime and compiler, or is a bytecode-only + interpreter iOS build preferable for most apps?
- Is an LLVM- or Cranelift-based AOT top tier worth a second backend? Compile time does not matter for AOT, but deopt and stackmap support is limited (Cranelift has no ValueRep-style deopt maps).

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/results-all.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/richards-results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/nbody-timed-results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/nbody-hyperfine.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/size-results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/richards-untyped.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/sh/richards-typed.c
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8size/all.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8size/agg.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8size/meta-summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8size/printopt.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/v8sizes.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/v8meta.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/build_sh.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/run_richards.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/run_nbody_timed.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/scripts/build_lean.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/hermes-build/bin/shermes
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/graal-DeoptimizationUtils.java
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/dl/art-inliner.cc
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/dl/v8-code-serializer.cc
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/dl/wasmtime-code_memory.rs
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8-deoptimization.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/v8-protectors.h
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/dynpgo-tiers.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-arch/r2r-overview.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/hermes
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/repos/dart-sdk
