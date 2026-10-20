# dual-mode-vms

## Summary
Every production VM that ships both modes uses ONE optimizing compiler and IR for both. The modes differ only in where speculation facts come from and in whether deopt exists:
- JIT: runtime feedback plus deopt.
- AOT: static proof (sound types, whole-program analysis), training-run profiles, or ML-guessed profiles. Speculation uses guard plus fallback, never deopt.

Examples: Dart (kernel to flow-graph IL; JIT speculates and deopts, AOT uses TFA and no deopt), Graal (libgraal JIT and native-image), RyuJIT (tiered JIT, ReadyToRun via crossgen2, NativeAOT), ART (one optimizing compiler for JIT and dex2oat), HotSpot Leyden JEP 544 (AOT code from the same C1/C2), Hermes (one IR to a bytecode or C backend), V8 (builtins compiled at build time by its own compiler pipeline).

Peak-performance data, all from primary sources plus my own runs on an M5 Max:
- **Where JIT wins.** Call sites that are monomorphic or low-polymorphic in practice but not provably so:
  - Dart TypedDataPoly with 2-5 receiver types: AOT/JIT 1.94 geomean, up to 3.6.
  - Dart Iterators mono: 1.25, up to 3.6.
  - Dart Richards: 1.28.
  - .NET Dictionary under NativeAOT: 2.1-5.3x slower, attributed by .NET runtime engineers to PGO-driven inlining and guarded devirtualization (GDV).
  - Dart rasterizer issue (2026): 6.5x slower in AOT, from a dispatch-table call that AOT does not inline.
- **Where AOT ties or wins.** Megamorphic sites (Dart AOT/JIT 0.50-0.69, thanks to its global dispatch table), call overhead in non-inlined code (0.67), provably monomorphic code (0.85), and closed small programs with a training profile. On AWFY, Graal native-image with PGO was 0.91x C2's time (faster) and 0.97x Graal JIT's.
- **Without profiles, AOT runs at about C2 speed or below.** Native-image O2 (ML-inferred profiles) was 1.02x C2 and 1.10x Graal JIT. Json/List stayed 1.3-1.5x slower than C2.

AOT reliably wins on:
- Startup and warmup: AOT first iteration ≈ its steady state; C2's first iteration is 2.07x its steady state (geomean), up to 8x. Leyden cut javac startup 33%.
- Memory: native-image median max RSS 57 MiB vs 870 MiB for C2 (heap defaults differ). Oracle reports 641 vs 1029 MB.
- Install and code size, when only hot code is precompiled (ART profiles about 20% of methods).
- Predictability.
- Platforms that forbid JIT (iOS outside BrowserEngineKit).

The industry trend is hybrid, not AOT-only:
- ART moved from full AOT to interpreter + JIT + profile-guided AOT.
- .NET recompiles hot ReadyToRun (R2R) code into tier-1 PGO code.
- Leyden JEP 544 says an AOT-only mode is a non-goal and that "minimizing JIT compilation often leads to lower peak performance".
- Hermes mixes native, bytecode and JIT code in one runtime.

Hermes's lead says untyped JS compiled AOT "isn't a performance improvement over a high tier JIT like JSC or v8"; even with types, "the win isn't guaranteed". In Serrano's Hopc study, full-AOT JS ran 1-2x slower than V8 on most JetStream tests.

## Recommendation
For ep9's AOT+JIT idea, copy the shared-optimizer pattern and make no broad claim that AOT beats V8 at peak.

1. Build one SSA IR and optimizer with pluggable speculation sources, not two compilers:
   ```
   source → bytecode ─┬─ interp ─ baseline ─ opt-JIT    (facts: ICs/feedback; guard → deopt to interp)
                      └─ AOT (build time) ─ same opt     (facts: TS types + whole-program analysis + training profile;
                                                          guard → in-code fallback; no deopt unless a JIT is present)
   ```
   This is what Dart, Graal, RyuJIT, ART, Leyden and Hermes all do.

2. Treat AOT code as a tier, not the top tier. In JIT-enabled mode, hot AOT functions get instrumented and recompiled with runtime feedback, as .NET R2R → tier1-instr → tier1-PGO and Leyden JEP 544 do. In jitless mode (iOS, locked-down hosts), AOT is the top tier. Use only guard plus fallback there, as ART does; the reason is ART's 'forever deopt' trap.

3. Share one value representation (NaN-box), one frame layout and one IC/feedback layout across the interpreter, baseline, AOT and opt code, as Hermes SHStackFrame and V8 Sparkplug do. AOT and JIT functions then call each other through the ordinary call path, and the interpreter or AOT baseline can be the deopt landing pad (SubstrateVM pattern).

4. Use TS types for what the data shows they are good at:
   - Unboxed numerics and field layout (Hermes typed: 7.5x+ on micros).
   - Proving monomorphism (Dart mono AOT/JIT 0.85).
   - A global dispatch table for class methods, which beats JIT on megamorphic sites (Dart 0.50-0.69).
   - Seeding speculation.

   Do not expect types to replace runtime feedback for sites that are polymorphic in practice: JIT is 1.3-3.6x faster there (Dart), 2-5x on .NET NativeAOT, and static devirtualization reaches only 5-15% of sites even with sound nominal types. TS types are unsound and structural, so typed fast paths need entry checks at untyped boundaries (Hermes checked casts).

5. Plan AOT-vs-JIT boundary costs: keep AOT code near JIT code (V8 short-builtin-calls lesson), and let the JIT inline AOT-compiled callees from IR, not as opaque calls (V8 had to re-implement builtins in js-call-reducer).

6. Make training profiles optional, and make sure feedback gathered at runtime can supersede them. My Leyden Havlak run showed that replayed profiles gave a 31% faster first iteration but a 20-40% slower steady state.

7. Position the product claim this way:
   - AOT wins on startup, time-to-peak, memory (~15x lower RSS), install size, predictability and jitless platforms.
   - At steady state, expect parity with a top-tier JIT on typed, closed, monomorphic or numeric code, and a 1.2-3x loss on polymorphic, library-heavy or regexp/eval code, unless the JIT tier is kept above AOT.
   - 'Faster than V8 TurboFan' is supportable only per workload class, with a training profile, or for short runs.

## Findings
- [high] Dart VM: AOT reuses the JIT pipeline, but JIT speculates from the execution profile and deoptimizes, while AOT runs global type flow analysis (TFA) plus tree shaking and compiles everything 'without any speculative optimizations'. Calls on dynamic receivers use switchable calls; all other calls go through a global dispatch table. The Dart VM author states: 'Currently Dart VM JIT has best peak performance, while Dart VM AOT has best startup time' and 'AOT compiled code reaches its peak performance almost immediately'.
  EVIDENCE: https://mrale.ph/dartvm/ (sections on the AOT pipeline, deoptimization, TFA, and method calls/dispatch table).
- [high] Dart team engineers give concrete reasons AOT loses at peak. (1) Regexps run on a bytecode interpreter in AOT because patterns may only be known at runtime. (2) AOT inlines conservatively to limit code size: --optimization-level=3 gave about 10% speed for 26% more code (5.7 to 7.2 MB). (3) AOT has no speculative inlining of call sites that are monomorphic in practice (e.g. String.codeUnitAt, which in practice only sees OneByteString) and no feedback-based polymorphic inlining. In 2019, dart_style was 65% slower in AOT; by Dart 2.10 the gap was 6.87 ms AOT vs 5.90 ms JIT. A 2026 issue shows a typed-data rasterizer 6.48x slower in AOT (3.37 vs 0.52 ms); the cause was a Uint32List.fillRange dispatch-table call to []= that JIT inlines and AOT does not.
  EVIDENCE: gh issue view 37774 -R dart-lang/sdk (mkustermann comments; mit-mit Dart 2.10 numbers); gh issue view 62619 -R dart-lang/sdk (alexmarkov diagnosis, opened 2026-02-08, still open).
- [medium] My measurement, Dart 3.13.5 on an M5 Max, benchmark_harness steady state, median of 3 interleaved reps, AOT/JIT time ratio (>1 means JIT faster). The result depends on the degree of polymorphism:
- TypedDataPoly with 2-5 receiver types: 1.94 geomean (1.37-3.62).
- Iterators.mono: 1.25 (up to 3.62).
- Richards: 1.28 at low load (JIT 251 us, AOT 322 us); AOT at --optimization-level=3 ran 257 us (1.02), and its snapshot grew 13% (932,768 to 1,054,192 bytes).
- SHA256 1.07, MD5 1.04, SkeletalAnimation 0.95.
- Provably monomorphic TypedDataPoly.mono: 0.85.
- Megamorphic: TypedDataPoly.mega 0.50, Iterators.poly 0.69 (AOT's global dispatch table beats JIT's megamorphic call path).
- Dynamic, a call-overhead microbenchmark with never-inline: 0.67.
- Binary sizes: dartvm (JIT VM) 41.5 MB vs dartaotruntime 4.8 MB; a Richards AOT exe is 5.7 MB.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dartbench/dart_jit_vs_aot.log, dart_summary.txt and richards_o3.txt (same dir). Runner: run.sh; analyzer: ../analyze_dart.py. The machine was shared; load average reached about 70 on 18 cores during part of the run, so treat single-benchmark ratios as ±15%. The polymorphism pattern is a 2x effect and holds across reps.
- [medium] Dart's AOT runtime gained an escape hatch for code that is not in the closed world: dart2bytecode emits bytecode for 'dynamic modules', executed by an interpreter inside the VM/AOT runtime (experimental). This mirrors Hermes's AOT-plus-interpreter mix.
  EVIDENCE: gh api repos/dart-lang/sdk/contents/pkg/dart2bytecode/docs/bytecode.md ('Dart bytecode is a binary representation of dynamic modules on the VM/AOT ... executed by the VM and AOT runtime'); runtime/vm/interpreter.cc is 180 KB; issue 63381 (2026-05) is about the KBC interpreter in vm/aot.
- [high] Oracle GraalVM's own numbers (Spring PetClinic, 512 MB heap, G1, native-image with PGO vs GraalVM CE with C2):
- Throughput: 11,902 vs 11,066 req/s.
- Startup: 0.21 vs 7.09 s.
- RSS: 641 vs 1029 MB.
- p99 latency: 5.15 vs 7.20 ms.
On the earlier Java 20 release, native-image was 'still slightly below JIT', and Oracle's native-image peak was 1.6x that of CE native-image. ML-based profile inference (no training run) gives about 6% runtime speedup over default native-image on Renaissance and DaCapo.
  EVIDENCE: https://blogs.oracle.com/graal/oracle-graalvm-for-jdk-21 (2023-09-19; scraped to .../dual-mode-vms/web/oracle-graalvm-jdk21.md); https://medium.com/graalvm/a-new-graalvm-release-and-new-free-license-4aab483692f5 (scraped to web/graalvm-2023-06.md). These are vendor benchmarks on one app, and the comparison is PGO-AOT vs C2, not vs Graal JIT.
- [medium] My measurement, Oracle GraalVM 25.0.4, AWFY Java (10 benchmarks), 150 iterations, steady state = median of the last 75, 5 reps pooled. Geomean time vs HotSpot C2:
- Graal JIT: 0.92.
- native-image O2 (default, with ML-inferred profiles): 1.02.
- native-image with PGO (O3, training-run .iprof): 0.91.
Against Graal JIT (same compiler): O2 1.10, PGO 0.97.
- AOT lost on Json (PGO 1.43-1.48 vs C2) and List (1.23-1.33). Graal JIT was also slower than C2 there (1.28-1.30, 1.19-1.20), so this is compiler quality, not mode.
- PGO won most on DeltaBlue (0.61-0.64) and Richards (0.67-0.71).
- First iteration over steady state: C2 2.07x geomean (DeltaBlue 8.2x, Havlak 4.0x); Graal JIT up to 12x; native-image 0.9-1.4x.
- Median max RSS: C2 870 MiB, Graal JIT 957 MiB, native-image 57-58 MiB (default heap policies differ).
- Binaries: 6.7 MB (O2), 6.3 MB (PGO), 43 MB (instrumented). Builds took 12-26 s.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/awfy_results_all.txt, awfy_summary.txt, awfy_summary2.txt; scripts run_awfy.sh and build_ni.sh; build logs in ni/build-*.log ('optimization level: 2 ... PGO: ML-inferred' and 'optimization level: 3 ... PGO: user-provided'). The PGO profile came from the same benchmarks, so it is a best case. Rep-to-rep spread had median 11% (max 36%). A second pass at load about 9 reproduced the geomeans (0.93, 1.02, 0.92).
- [high] Native-image AOT code is not speculative or deoptimizable. In SubstrateVM, the source of a deoptimization is always runtime-compiled code (Truffle JIT inside the image) and the target is always an AOT-compiled method. So AOT code serves as the deopt target for JIT code, which is a reusable pattern: an AOT-compiled interpreter or baseline acting as the deopt landing pad.
  EVIDENCE: oracle/graal substratevm/src/com.oracle.svm.core/src/com/oracle/svm/core/deopt/Deoptimizer.java class comment: 'The method to deoptimize (= the source method) is either a specialized runtime compiled method or an AOT-compiled test method ... The target method is always an AOT-compiled method.'
- [high] Project Leyden status (openjdk.org, page updated 2026-10-01):
- Delivered: JEP 483 AOT class loading and linking (JDK 24), 514 command-line ergonomics (JDK 25), 515 AOT method profiling (JDK 25), 516 AOT object caching with any GC (JDK 26).
- JEP 544 (AOT code compilation): Status Integrated, Release 28.

JEP 544 design: AOT code is produced by the same C1/C2 in a training run, coexists with JIT code, and falls back to the interpreter/JIT on deopt. It requires the same CPU features and the same GC (barriers are baked into the code). Static finals cannot be constant-folded, and class-init checks need a slow version and a fast version.
- Results: startup reduced 65-80% with AOT code vs 50-70% without.
- Non-goal: 'an AOT-only mode'.
- Future work: 'minimizing JIT compilation often leads to lower peak performance'.
- JEP 515: 'A partial AOT solution, where reasonable AOT code is gradually replaced by better-optimized JIT code, seems likely to be the best solution in the end.'
- JEP 544 also concedes static compilation 'can even be competitive with HotSpot if ... guided by accurate profiles'.
  EVIDENCE: https://openjdk.org/projects/leyden/ ; https://openjdk.org/jeps/544 ; https://openjdk.org/jeps/515 ; https://openjdk.org/jeps/483 (saved as .../dual-mode-vms/web/jep*.txt). JEP 483: PetClinic startup 4.486 to 2.604 s with a 130 MB cache.
- [medium] My measurement, JDK 27 (Homebrew; has JEPs 483/515, not 544):
- AOT cache (classes + training profiles) cut javac startup plus one compile of 79 files from 0.51 to 0.34 s (-33%), and a 20-iteration javac loop from 1.85 to 1.39 s (-25%). Cache size 34 MB.
- On compute-heavy AWFY Havlak, the first iteration fell 99 to 68 ms (-31%), but steady state over iterations 60-120 got 20-40% slower (25.6-27.7 ms to 32.6-38.5 ms).
- With -XX:-AOTReplayTraining (cached classes, no replayed profiles), steady state returned to 26.8-27.4 ms and the first-iteration gain mostly vanished (92-98 ms).
- Conclusion: profile replay sped up warmup but locked in worse steady-state code for this workload.
  EVIDENCE: /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/leyden/leyden_results.txt, leyden_summary.txt, havlak120_results.txt, run_leyden.sh. n=3-5. Shared, noisy machine. Root cause not investigated beyond the AOTReplayTraining toggle.
- [high] .NET:
- ReadyToRun docs: 'Ahead-of-time generated code is not as highly optimized as code produced by the JIT. To address this issue, tiered compilation will replace commonly used ReadyToRun methods with JIT-generated methods.' R2R assemblies are 2-3x larger.
- .NET 8 added an R2R path: R2R (optimized, uninstrumented) → tier1-instrumented → tier1 with PGO.
- Dynamic PGO, on by default since .NET 8, gives about 15% on average; 23% of about 4,600 benchmarks improve by 20% or more.
- RyuJIT is the code generator for JIT, crossgen2/R2R and NativeAOT.
  EVIDENCE: https://learn.microsoft.com/en-us/dotnet/core/deploying/ready-to-run ; https://learn.microsoft.com/en-us/dotnet/core/whats-new/dotnet-8/runtime ; https://devblogs.microsoft.com/dotnet/performance-improvements-in-net-8/ (dotnet/runtime#70941, #86225); dotnet/runtime src/coreclr/nativeaot/docs/compiling.md:335 ('RyuJIT is used as the code generator to produce the ReadyToRun images').
- [high] .NET NativeAOT vs tiered JIT with PGO at peak (dotnet/runtime#129895, 2026, closed as not planned; .NET 10 on an Apple M2):
- Dictionary foreach: 828 ns JIT vs 2,125 ns NativeAOT (2.6x).
- ForeachEnumerable: 835 vs 4,440 ns (5.3x, and NativeAOT allocates).
- GetString: 3.35 vs 7.19 ns (2.1x).
Runtime team members explain it: 'benefits of super aggressive inlining triggered by tiered compilation with PGO data'. PGO devirtualizes the comparer's GetHashCode/Equals and constant-folds the hash. JIT with TieredCompilation=0 reproduces the NativeAOT number (GetString 4.1 to 7.7 ns; 5.9 ns with class profiling off).
Microsoft's ASP.NET 8 NativeAOT launch post gives startup -70%/-76%, size -89%/-78% and working set -57%/-37%, and reports no throughput number.
  EVIDENCE: gh issue view 129895 -R dotnet/runtime --comments (jkotas, EgorBo, hamarb123 tables); https://devblogs.microsoft.com/dotnet/announcing-asp-net-core-in-dotnet-8/ (tables: 156 → 48 ms, 92.6 → 10.0 MB, 96 → 41 MB).
- [high] Static devirtualization is weak even with sound nominal types. The .NET JIT design doc says only about 15% of virtual call sites and about 5% of interface call sites can be devirtualized by analysis. Guarded devirtualization instead guesses a class from profile data, tests it, calls directly, and keeps the original virtual call in the else branch, with no deopt.
  EVIDENCE: dotnet/runtime docs/design/coreclr/jit/GuardedDevirtualization.md, lines 15-50.
- [high] Android ART:
- Android 5-6 used full AOT: 'long wait times to use the app, increased RAM and disk space usage, and longer times to load code from disk'.
- Android 7 added interpreter + JIT + profile-guided AOT, compiled when the device is idle and charging. Apps that 'required several minutes to optimize and install in Android 6.0 can now install in just a matter of seconds'.
- JIT and AOT use the same compiler, but JIT uses runtime type info, inlines better and supports OSR. When both JIT and AOT code exist, the JIT code is preferred.
- Cloud profiles: about 15% faster cold start on average (YouTube 20%+, Search 30%); about 20% of methods profiled (2-60%). Precompiled code is clean memory, while JIT code is dirty memory.
- Baseline Profiles: about 30% faster code execution from first launch.
  EVIDENCE: https://developer.android.com/about/versions/nougat/android-7.0 ; https://source.android.com/docs/core/runtime/jit-compiler ; https://android-developers.googleblog.com/2019/04/improving-app-performance-with-art.html ; https://developer.android.com/topic/performance/baselineprofiles/overview (table of compilation behavior by Android version).
- [high] ART's AOT compiler uses offline inline caches from profiles (receiver types per dex pc, megamorphic flag), but deliberately emits polymorphic inlining with a fallback and no deopt. Its stated reasons: a deopt in AOT code may never refresh the profile, leading to 'forever deoptimizations'; and for processes 'not allowed to JIT ... we will run interpreted code for the rest of the lifetime'. The JIT, by contrast, uses runtime inline caches and deopt.
  EVIDENCE: android.googlesource.com platform/art compiler/optimizing/inliner.cc HInliner::UseOnlyPolymorphicInliningWithNoDeopt and TryInlineFromInlineCache (GetInlineCacheAOT vs GetInlineCacheJIT); libprofile/profile/profile_compilation_info.h (inline cache encoding).
- [high] V8 is AOT inside a JIT engine. All builtins and bytecode handlers are compiled at build time by V8's own compiler pipeline (mksnapshot writes embedded.cc as .byte directives into .text), shared across isolates and processes. This cut median V8 heap by 19%. It first cost Speedometer2 5%, reduced to about 0.5% via root-register-relative loads and pc-relative builtin-to-builtin calls. I measured node 24.22 / V8 13.6 arm64: 2,384 Builtins_* symbols in .text, including 515 bytecode-handler variants (normal/Wide/ExtraWide) and 82 *_Baseline builtins called by Sparkplug code. Embedded blob: 1.87 MiB code + 0.10 MiB data.
  EVIDENCE: https://v8.dev/blog/embedded-builtins ; nm on /Users/brooklyn/.vite-plus/js_runtime/node/24.22.0/bin/node, with v8_Default_embedded_blob_code_size_ = 1,958,784 read from __DATA_CONST; symbol list in .../dual-mode-vms/v8-builtins.txt.
- [high] Calls between AOT and JIT code have a placement cost. V8 9.1 temporarily un-embedded builtins on desktop: JIT code calling builtins more than 4 GiB away causes indirect-call mispredictions (Apple M1 never predicts targets beyond 4 GiB; arm64 direct calls reach only 128 MiB). So V8 copied builtins into the pointer-compression cage at +1.2-1.4 MiB per instance. Also, TurboFan re-implements key builtins as inline graph reductions (js-call-reducer.cc: ReduceArrayMap, ReduceArrayForEach, ReduceArrayFilter, ReduceArrayPrototypePush), because a call into an AOT builtin is opaque to the optimizer.
  EVIDENCE: https://v8.dev/blog/short-builtin-calls (2021-05-06); gh api repos/v8/v8/contents/src/compiler/js-call-reducer.cc (414 KB; lines 4117, 4190, 4217, 6585).
- [high] V8's code cache stores bytecode, not machine code. Caching after top-level execution cut parse and compile time by 20-40%. Ignition bytecode is '50% to 25% the size of the equivalent baseline machine code'. Prior research also found V8 --jitless costs about 40% on Speedometer2 and about 80% on Web Tooling, and measured 1.33x QuickJS on Octane vs 25.6x for full V8.
  EVIDENCE: https://v8.dev/blog/code-caching-for-devs ; https://v8.dev/blog/improved-code-caching ; https://v8.dev/blog/ignition-interpreter ; .../scratchpad/research/execution-tiers.md lines 90, 113, 159.
- [high] Hermes (static_h branch, now Hermes V1):
- Compile modes: typed or untyped JS, AOT to bytecode or to native (C backend lib/BCGen/SH/SH.cpp, then the system cc).
- Runtime modes: native, interpreter, lazy compile, baseline JIT, all 'mixed and exist in the same runtime simultaneously'.
- Interop mechanism: natively compiled functions are ordinary closures created by _sh_ljs_create_closure with a C function pointer. They use the same NaN-boxed SHLegacyValue and the same register-stack frame layout (SHStackFrame: PreviousFrame, SavedIP, SavedCodeBlock, ArgCount, NewTarget, CalleeClosureOrCB, ThisArg, args) as the interpreter, and _sh_ljs_call dispatches uniformly.
- Typed mode is a sound subset: `any` casts to other types with a runtime check; unannotated functions are untyped and their calls are not type-checked.
  EVIDENCE: facebook/hermes static_h: doc/blog/2025-11-02-hermes-compilation-runtime-modes.md; include/hermes/VM/static_h.h (_sh_ljs_call line 492, _sh_ljs_create_closure line 567); include/hermes/VM/sh_stack_frame.h; doc/TypedLanguage.md; discussion #1685 (avp: SH.cpp is the C backend). Copies in .../dual-mode-vms/web/.
- [high] Hermes data on untyped AOT:
- interp-dispatch micro: native AOT of untyped code ran 2388 ms vs the interpreter's 2086 ms (slower); every op stays a runtime call because types infer only as number|bigint.
- One `fc = +fc` coercion: 278 ms (7.5x).
- With inlining and constant propagation: 6 ms.
Hermes lead (2025): 'compiling untyped JS to native AOT isn't a performance improvement over a high tier JIT like JSC or v8. Even with typed code, the win isn't guaranteed. It is more about getting predictable performance than being faster in absolute terms.' Static_h's JIT on untyped Octane: 1.15-2.07x over the old Hermes interpreter; type-inferable micros: 3.3-7.9x.
  EVIDENCE: https://tmikov.blogspot.com/2023/09/how-to-speed-up-micro-benchmark-300x.html ; github.com/facebook/hermes/discussions/1685 (tmikov reply, 2025-04-24); doc/blog/2024-12-19-static-hermes-update-dec-2024.md table.
- [high] Full-AOT JavaScript with no annotations (Serrano, Hopc, ICFP 2021; compared against V8 6.8, JSC, SpiderMonkey 68):
- Most JetStream tests run 1-2x slower than V8; the call micro was about 2x slower than V8.
- Weak on: deltablue (small methods that JITs inline away), boxed floating point (crypto, n-body, raytrace), regexp, and eval (it 'cannot deliver good performance for programs relying on eval ... requires either a JIT compilers or a hybrid AoT and JIT approach').
- On 18 new, more diverse benchmarks it was competitive in speed and used significantly less memory.
  EVIDENCE: https://www-sop.inria.fr/members/Manuel.Serrano/publi/serrano-icfp21.pdf (text at .../dual-mode-vms/dl/papers/serrano-icfp21.txt, Section 6.1, around lines 1365-1410).
- [medium] Brief notes on other systems:
- PyPy: the interpreter is written in RPython and AOT-translated; the tracing JIT is generated from the same interpreter with hints ('same language semantics as the original interpreter by construction'). One semantic source feeds both modes.
- Julia: JIT at first call via type inference and LLVM. 1.9 caches native code in package images (precompile time +10-50%). 1.12 adds an experimental --trim/juliac that requires no reachable dynamic dispatch.
- LuaJIT 2.1, my measurement on AWFY Lua: interpreter-only (-joff) 2.88x slower than JIT (geomean). Richards hit a JIT cliff at inner=100 (5.17 s JIT vs 1.66 s interpreter), while inner=20 was normal (103 vs 370 ms). Single run, low confidence.
  EVIDENCE: https://rpython.readthedocs.io/en/latest/jit/overview.html ; https://julialang.org/blog/2023/04/julia-1.9-highlights/ ; https://julialang.org/blog/2025/10/julia-1.12-highlights/ ; .../dual-mode-vms/luajit/luajit_summary.txt.
- [medium] iOS: JIT for third-party code is limited to alternative browser engines via BrowserEngineKit entitlements (EU on iOS 17.4+, also Japan). An embedded JS engine in a normal iOS app must run jitless, using an interpreter and/or AOT. This is the main reason Hermes and Dart AOT exist.
  EVIDENCE: https://developer.apple.com/documentation/browserenginekit (via search summary); Dart: 'AOT snapshots were originally introduced for platforms which make JIT compilation impossible' (mrale.ph/dartvm).

## Risks
- My experiments ran on a shared M5 Max where other agents and processes pushed load average to about 70 on 18 cores during parts of the runs. Runs were interleaved and repeated, with medians and a second AWFY pass at load about 9, but single-benchmark ratios carry about ±15% noise; only geomeans and 2x-class effects are robust.
- The AWFY PGO profiles were trained on the same benchmarks used for measurement (best case for AOT). Real-world training/production mismatch would lower native-image PGO results.
- Native-image and HotSpot RSS comparisons use default heap policies (JVM max heap = 1/4 of 128 GB; Serial GC in native-image on macOS). They show the mechanism but are not an equal-heap comparison.
- Vendor numbers (Oracle PetClinic; Microsoft ASP.NET; Google cloud profiles) come from single apps chosen by the vendor. Microsoft's ASP.NET NativeAOT post reports no RPS at all.
- Serrano's Hopc compared against V8 6.8 (2018). Modern V8 (Maglev, Turboshaft) likely widens the gap, so '1-2x slower than V8' is optimistic for full-AOT JS today.
- The Leyden Havlak steady-state regression from replayed profiles is one workload on JDK 27, root cause unconfirmed. It may be fixed in JDK 28 with JEP 544.
- The Dart Dynamic, Iterators and TypedDataPoly results are microbenchmarks made to isolate dispatch costs. The overall 0.85 Dart geomean is dominated by them and should not be read as 'Dart AOT is faster overall'.
- Hermes typed-mode wins come from tmikov micro-benchmarks. I found no published Hermes typed-native vs V8/JSC comparison on a realistic app.

## Open questions
- How much of the Dart and .NET AOT peak loss could a TS-type-seeded speculation plus guard-and-fallback scheme (ART-style polymorphic inline with no deopt) recover without any training profile? No system publishes this ablation.
- Can ep9 safely use training-run profiles for AOT JS if an IC/feedback layout is shared, and what is its invalidation story when production shapes diverge (the Leyden Havlak case)?
- Should ep9 build a JIT-enabled hybrid mode, where AOT code is recompiled by the opt-JIT? The Leyden/R2R/ART data says it beats both pure modes, but it doubles the testing matrix.
- Equal-heap memory comparison of AOT vs JIT for a JS engine: how much of native-image's RSS win comes from not shipping the compiler and profiling metadata, and how much from heap policy? This needs a controlled run.
- Is a hand-written Rust interpreter with optional AOT-compiled TS modules enough for iOS, or does ep9 need an AOT-generated baseline interpreter (V8 embedded handlers or JSC LLInt style) to keep jitless mode within 1.3-1.5x QuickJS-class speed?
- Code-size budget: Dart's O3 inlining gave +10-25% speed for +13-26% size. What inlining/size policy fits ep9's AOT output when shipping per-app native code alongside the engine?

## Artifacts
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dartbench/dart_jit_vs_aot.log
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dartbench/dart_summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dartbench/richards_o3.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dartbench/run.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/awfy_results_all.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/awfy_summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/awfy_summary2.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/run_awfy.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/awfy/build_ni.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/leyden/leyden_results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/leyden/leyden_summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/leyden/havlak120_results.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/leyden/run_leyden.sh
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/luajit/luajit_summary.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/v8-builtins.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/analyze_awfy.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/analyze_dart.py
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/dl/papers/serrano-icfp21.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/web/jep544.txt
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/web/oracle-graalvm-jdk21.md
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/web/hermes-static_h.h
- /private/tmp/claude-501/-Users-brooklyn-workspace-github-ep9/53ce3b1c-bc15-4f01-a4d1-8e97c5516415/scratchpad/work/dual-mode-vms/web/hermes-blog-2025-11-02-hermes-compilation-runtime-modes.md
