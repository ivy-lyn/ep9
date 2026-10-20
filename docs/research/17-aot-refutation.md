# AOT synthesis: adversarial review

## Overall
The synthesis's main recommendation holds. Make no general 'AOT beats V8' claim, let the JIT own peak on general JS, treat TS types only as guarded hints, keep tsrs as an offline oracle in a separate binary, and use one optimizer with per-mode guard policy. But several supporting claims are overstated or misattributed.

Refuted:
- (a) 'In all dual-mode VMs the JIT owns peak'. Graal PGO native-image beat its own JIT (0.892 vs 0.918 of C2). In Hermes, typed AOT is the fastest mode and its JIT is a baseline with no runtime fact source.
- (b) Startup (1.5-5 ms vs node 23.5 ms) and memory (10-17 vs 50-62 MB) as AOT wins. In the same data, QuickJS running from source gets 1.99 ms, bun (JIT) gets 5.77 ms and 10.5 MB, and Hermes JIT RSS equals Static Hermes native RSS. These are runtime effects, not mode effects.
- (c) The .NET 15%/5% static-devirtualization figure as a sound-type bound. It describes the early, weak local analysis of the .NET Core 2.0 JIT.

Weakened:
- scriptc '1.4x': the upper end uses V8 on a different source. Same-source is 1.25-1.28x including V8 warm-up, and about 1.15x at peak.
- The '1.22x ceiling' rests on one FP kernel.
- The '0-5% at peak' bound ignores StrongScript's +22% and the ~8% check-cost figure.
- 'TS addresses ≤33% of deopts' ignores about 10% field-type lazy deopts, and counts events, not time.
- '24/24' and '13/13' are crafted suites, so 100% is true by construction.
- 'Untyped AOT ≈ baseline JIT' ignores Hopc, which reached 1-2x of V8.
- The JSC no-cliff cells cite the wrong rows; the correct figures are 237 vs 231 ms.
- Hybrid 'feedback keeps flowing' conflicts with .NET's 'never instrument R2R'.
- D5 shape determinism is not required if AOT guards compare against RW cells, as Static Hermes does.

Design gap: the span join was validated only on the parse tree. oxc's TS lowering ([[Set]] fields, parameter properties, enums) builds nodes with dummy spans, and bundled output breaks the join. Hints must be attached before the transform, or the lowering must move into BytecodeGen.

Net: keep the architecture and the A4 kill criterion. Restate the AOT value claim as 'startup/warm-up, jitless platforms, predictable latency, and possibly total deployment size'. Memory and hello-world startup gains must be measured on equal runtimes before ep9 uses them as AOT selling points.

## Verdicts
- [weakened] Sound-typed JS AOT (Static Hermes) ran 1.5-3.2x slower than V8 full (geomean ~2.1x), so typed AOT faster than V8 is not supported
  - Evidence: The numbers check out: aot-vs-jit/results-table.md shows 173/90, 288/91 and 148/96, geomean 2.105. But they measure Static Hermes's immature backend, not a ceiling for typed AOT. repos/facebook_hermes/benchmarks/octane/richards-typed/optimization-notes.md:104-165 lists its gaps: NaN-boxed number and 'does not yet support native integer types' (line 119), only 5 direct slots, redundant CheckedTypeCastInst, and little inlining. _sh_fastarray_load is an out-of-line call. On the same nbody, scriptc (struct layout, unboxed f64) ran 157 ms per 5M steps, which is about C -ffp-contract=off speed (74 ms per 2.5M = 148 per 5M).
  - Correction: Keep the measurement. Present it as 'today's only sound-typed JS AOT is 1.5-3.2x behind V8', not as a bound on what a typed AOT with int32, unboxed layout and inlining could reach.
- [weakened] Untyped AOT runs at baseline-JIT speed (10-13x slower than V8, ~1.6-2.0x its interpreter)
  - Evidence: This holds for Static Hermes and Porffor (results-table.md; octane-table.md: shermes-native 0.094 vs hermes-jit 0.117). Hopc is counter-evidence the synthesis does not use for speed. It is untyped AOT with inference, optimistic guards and static ICs, and aot-precedents/docs/icfp21.txt:1389 says 'For many tests, Hopc performance is in the range 1x..2x when compared to V8' (V8 6.8). Its conclusion (line 2122) says whether AOT can compete on speed 'is still unanswered'. The interpreter ratio is also too high at the bottom end: raytracer is 1558/1186 = 1.31x.
  - Correction: Say 'naive untyped AOT (generic runtime calls) ≈ baseline JIT'. A whole-program inference AOT (Hopc) reached 1-2x of 2018 V8 on most JetStream tests. Hermes untyped native gives 1.3-2.0x its interpreter.
- [weakened] scriptc ran nbody 1.15-1.4x faster than V8 (bit-identical)
  - Evidence: aot-precedents/nbody-results.txt shows V8 223 ms on nbody.js, which is the original object-literal source and not the class source scriptc compiled. On the same class source (nofields), V8 ran 196-201 ms. scriptc took 157 ms (scriptc-results.txt; internal Date.now, single shot, so V8's warm-up is included). V8's in-process peak is 90 ms per 2.5M steps (results-table.md), about 180 per 5M. The 1.4x upper bound also conflicts with the synthesis's own '1.22x ceiling'.
  - Correction: About 1.15x vs V8 peak and 1.25-1.28x including V8 warm-up, on the same source. Drop 1.4x.
- [weakened] C that keeps JS float rules is only 1.22x faster than V8, so that is the ceiling
  - Evidence: This is one benchmark (nbody) with a pointer-to-heap-struct layout. The aot-perf-reality risks note that struct-of-arrays C would be faster. On richards and raytracer V8 is not even the fastest JS engine: bun runs 74 vs 91 and 77 vs 96 ms (results-table.md), so no ceiling for non-FP code is established.
  - Correction: Call it 'the ceiling on nbody with JS-shaped data'. No ceiling was measured for OO or dispatch code.
- [weakened] TS types are unsound: 24 of 24 strict programs gave wrong types; 13 of 13 tsrs hint holes gave a wrong hint
  - Evidence: I recounted 47 LIE lines in ts-type-trust/unsound/run.log. But both suites were written to exploit known holes, so 100% is true by construction. They prove unsoundness exists, not how often it happens in real code.
  - Correction: Say 'each of 24 known hole classes passes tsc/tsrs strict and lies at runtime'. Do not present 24/24 or 13/13 as a rate.
- [weakened] TS types cannot prevent 'wrong map' (49% of deopts); TS types address 33% of deopts at most
  - Evidence: I recounted ts-type-trust/tscheck/deopt.log: 1,693 deopts, wrong map 830, insufficient-feedback reasons 559. Another 156 'dependent field type constness changed', plus 3 field-type-changed and 2 field-representation-changed, are field-type lazy deopts. TS field types could seed these, which the synthesis itself proposes ('seed the field-type lattice'). Some wrong-map deopts come from map deprecation after field generalization. Deopt counts also are not time.
  - Correction: Say 'insufficient feedback was 33% and field-type dependencies about 10% of deopt events'. The share TS can address and the time it would save are unmeasured. Do not use '33% at most'.
- [weakened] TS types can only be guarded hints, worth about 0-5% at peak
  - Evidence: The sources span 2.2-4.6%: IISWC'16, Crankshaft-era V8, removing check branches only. The IISWC'21 figure is ~8% (abstract only). StrongScript gave -0.9% to +22.3% (navier +20.6, raytrace +22.3). None of these measure layout or representation effects. No guarded-hint system exists to measure.
  - Correction: Say 'unmeasured; guard-removal studies give 2-8%, sound types inside 2015 V8 gave -1% to +22%'. Keep the A4 kill criterion.
- [refuted] Every production dual-mode VM (Dart, Graal, .NET, ART, Leyden, Hermes) uses ONE optimizer with different fact sources, and in all of them the JIT owns peak speed
  - Evidence: Graal (dual-mode-vms/awfy/awfy_summary.txt) has steady-state geomean vs C2 of graaljit 0.918, ni-o2 1.011, ni-pgo 0.892. So PGO native-image beat its own JIT, though it was trained on the same benchmarks. Oracle PetClinic: PGO native 11,902 vs C2 11,066 req/s. In Hermes, typed AOT native is the fastest mode (nbody 173 vs hermes-jit typed 354). Its JIT is a baseline over bytecode with no runtime fact source and no deopt, so 'different fact sources' does not fit either. Dart's whole measured suite has AOT/JIT geomean 0.85 (dart_summary.txt), though micro-dominated. JEP 544 line 40: static compilation 'can even be competitive with HotSpot if ... guided by accurate profiles'.
  - Correction: Say 'in VMs with a speculative JIT, the JIT owns peak on polymorphic-in-practice code without a profile. With a representative training profile (Graal PGO), or when the JIT is only a baseline (Hermes), AOT matches or beats it.' Drop Hermes from the 'one optimizer, two fact sources' list.
- [refuted] AOT wins startup: hello world 1.5-5 ms vs node 23.5 ms (AOT/bytecode 5-16x faster than node)
  - Evidence: From aot-vs-jit/startup.json (I recomputed the medians): QuickJS 1.99 ms parses hello.p.js from source with no AOT and no bytecode. bun, a JIT, runs in 5.77 ms. Static Hermes native (4.87 ms) is slower than Hermes bytecode (4.03 ms). Node's 23.5 ms is Node bootstrap, which already uses a V8 snapshot, not the JIT.
  - Correction: Hello-world startup reflects runtime bootstrap and snapshot design, not AOT vs JIT. A Node-compatible ep9 pays its own bootstrap whatever the mode. Only the 'load 9 MB of code' rows say anything about bytecode caching.
- [refuted] AOT saves memory: peak RSS Static Hermes 10-17 MB vs node 50-62 MB (3-15x less)
  - Evidence: The same results-table.md has hermes-jit RSS 8.8-16.1 MB, the same as shermes-native at 9.8-17.0 MB, and hermes-interp at 8.5-15.6 MB. bun (JIT) hello is 10.5 MB (startup.json). The gap is V8/Node vs Hermes, not mode. native-image vs C2 compares different heap defaults, as the report admits. Only Hopc's ICFP'21 'at least 3x less memory than V8' (icfp21.txt:2116) is a mode-related data point.
  - Correction: Memory gain from AOT is unmeasured for JS on equal runtimes. What can be claimed is no feedback vectors, no JIT code and no compiler threads. Size it in ep9 itself.
- [weakened] Hermes .hbc 14.9 ms vs 836 ms from source is the largest measured startup gain, so A1 (bytecode cache) first
  - Evidence: In aot-precedents/startup-results.txt, 836 ms is Hermes's own slow -lazy source path (±208 ms). Other engines gain less: V8 compile cache 175.6 to 87.8 ms (2.0x), bun --bytecode 2.1x, qjsc 10x. For ep9 the gain must be measured against its own oxc-based source path.
  - Correction: Expect 2-10x on large bundles, depending on how fast the source path is. A1 is still a sensible first step, but 56x is engine-specific.
- [refuted] Static devirtualization reaches only ~15% of virtual and 5% of interface call sites, even with sound nominal types (.NET GuardedDevirtualization.md)
  - Evidence: gh api docs/design/coreclr/jit/GuardedDevirtualization.md, Motivation: the stats describe the .NET Core 2.0 JIT, whose 'analysis is somewhat weak ... places where types just get lost ... happens quite early (during importation)'. It says proving monomorphism 'would require sophisticated interprocedural analyses that are not practical in the JIT'. That is not a bound for whole-program or sound-type analysis such as NativeAOT or Dart TFA.
  - Correction: Cite it only as 'a local JIT analysis devirtualized 15%/5%'. No figure for whole-program devirtualization was measured.
- [upheld] .NET NativeAOT is 2.1-5.3x slower than tiered JIT+PGO on Dictionary code (dotnet/runtime#129895)
  - Evidence: gh issue view: 'Dictionary is much slower under NativeAOT than JIT', created 2026-06-26, closed NOT_PLANNED 2026-06-30. EgorBo attributes it to PGO devirtualization of comparer GetHashCode/Equals plus aggressive inlining.
  - Correction: 
- [weakened] Hybrid mode: AOT units are tier-0 optimized code, 'feedback keeps flowing', and the JIT re-optimizes (as in .NET R2R and Leyden)
  - Evidence: dual-mode-arch/dynpgo-tiers.md:97: 'we never instrument R2R'd code - that is a lot of additional size increase'. R2R code is promoted after 30 calls to a separate Tier1Instrumented re-JIT (lines 22, 99-107), and even that profile 'doesn't instrument inlinees' (145).
  - Correction: Optimized AOT code does not produce feedback for free. Either keep the data ICs in AOT code writing the shared feedback schema, or re-tier hot AOT functions through an instrumented tier. Budget for the size and complexity.
- [weakened] tsrs oracle: join on (start,end)+kind reaches 99.94%, and ep9 strips TS itself through oxc, applying useDefineForClassFields and decorator semantics
  - Evidence: The join was measured on the oxc_parser AST before any transform (tsrs-type-oracle/out/ts-compiler.report.json). oxc_transformer then builds lowered nodes with the dummy SPAN: in repos/oxc/crates/oxc_transformer/src/typescript/class.rs:442-471, the [[Set]]-mode field to `this.x = v` assignment and parameter properties use create_this_property_access(SPAN, ...). enum.rs:137-384 does the same. So exactly the class-field stores where shape and slot hints matter, in 4 of 6 [[Set]] codebases, lose their keys. Bundled or minified JS (most production deployments) also breaks the join unless source maps are used. The synthesis does not list this as a risk.
  - Correction: Attach hints to AST nodes before the transform and carry them through the lowering, or lower TS semantics in BytecodeGen instead of the oxc transformer. Add source-map-based joining for bundled output, or accept that bundles get no hints.
- [upheld] tsrs must stay a separate binary: unicode-id-start =1.1.2 pin and a fixed 32 GiB reservation at 0x4001_0000_0000
  - Evidence: third_party/tsrs/Cargo.toml:41 `unicode-id-start = "=1.1.2"`; crates/tsrs_core/src/reserve.rs:19 ('2^32 units: 32 GiB'), :46 `BASE_ADDR: usize = 0x4001_0000_0000`.
  - Correction: 
- [upheld] Hermes untyped native was slower than Hermes' own baseline JIT on every kernel
  - Evidence: results-table.md: nbody 980 vs 920, richards 1158 vs 990, raytracer 1186 vs 937. octane-table.md: shermes-native scores below hermes-jit on all 7 kernels (e.g. deltablue 5027 vs 7260).
  - Correction: 
- [weakened] V8's 5.7x `x;` class-field cliff is V8-specific; JSC shows no cliff (112 vs 114 ms)
  - Evidence: The conclusion holds, but the cited cells are wrong. In aot-vs-jit/results-table.md, bun 112 is nofields and 114 is untyped. There is no bun row for the 'stripped' source with `x;` fields. The real evidence is aot-precedents/nbody-results.txt: 'JSC full nbody-typed.stripped 237 ms' vs nofields 231 ms, against V8 1428 vs 201.
  - Correction: Cite JSC 237 vs 231 ms (5M steps) against V8 1184-1428 vs 196-201 ms.
- [weakened] Native code is 6-19x the bytecode, so JIT gives the smaller payload ('JIT for smaller size')
  - Evidence: This is true per app when the engine is installed separately. For self-contained deployments the JIT runtime itself is bigger: dartvm (JIT) 41.5 MB vs dartaotruntime 4.8 MB, and a Richards AOT exe is 5.7 MB (dual-mode-vms report). The ASP.NET NativeAOT post gives size -89%. The 6-19x ratio covers only code bytes.
  - Correction: Separate per-app payload (JIT smaller) from total shipped size: engine plus app, where AOT plus a lean runtime can be smaller. Measure both for ep9's embedding targets.
- [weakened] D5: AOT shape guards need deterministic, versioned transition trees and stable ShapeIds across build and runtime
  - Evidence: Static Hermes AOT C does not need this. It compares against RW per-site cache entries {clazz, slot} filled at runtime (dual-mode-arch/sh/richards-untyped.c:186-190; sh_mirror.h), and Dart uses data-held switchable calls. Only guards that embed a build-time ShapeId as an immediate need determinism.
  - Correction: Make the default AOT shape guard compare against a RW link-table or IC cell resolved at first execution (GOT-like). Require deterministic ShapeIds only for snapshot-created root shapes.
- [weakened] Every full-language AOT system (Hermes, Dart 2024, Graal Crema, Hopc, scriptc) keeps an interpreter for eval/new Function/toString
  - Evidence: Dart AOT shipped from 2020 to 2024 with no interpreter (bytecode removed in 7588ed86de), and native-image shipped for years without Crema. Neither language has eval. scriptc refuses eval by default; quickjs-ng is used only with --dynamic. The JS conclusion still holds through test262 (1,791 eval/Function( files).
  - Correction: Base D9 on test262 requirements, not on 'every AOT system'.
- [upheld] Meta (2026-10-09): untyped native tsc 1.20 s vs node 354 ms; binary 26.3 to 69.8 MB
  - Evidence: I fetched speakerdeck.com/tmikov2023/compiling-a-real-world-cli-tool-to-native-with-static-hermes. Slide 34 (Linux x86-64, not this Mac): 'Run time 1.27 → 1.20 s (−6%)', 'Size 26.3 → 69.8 MB', 'tsc on node (V8): 354 ms', 'native tsc starts in 18 ms, against node's 75 ms'. The talk is dated October 09, 2026.
  - Correction: Note that the platform is Linux x86-64.
- [upheld] Graal native-image PGO 0.89-0.92x C2; Leyden replayed profile made Havlak steady state 20-40% slower; JEP 544 'minimizing JIT compilation often leads to lower peak performance'
  - Evidence: awfy_summary.txt ni-pgo 0.892 vs C2. jep544.txt:107 has the exact quote, and :61 gives startup -65..80% with AOT code. The Havlak result is a single workload (medium confidence in the report).
  - Correction: '.NET R2R plus Leyden cut startup 65-80%' mixes sources: 65-80% is JEP 544 only. ASP.NET NativeAOT startup is -70/-76%.

## Missing counter-evidence
- Hopc (ICFP'21, icfp21.txt:1389, 2122) is untyped whole-program AOT with optimistic guards. It reached 1-2x of V8 on many JetStream tests, and the authors call AOT-vs-JIT peak 'still unanswered'. That directly undercuts 'untyped AOT ≈ baseline JIT' as a general law.
- Graal native-image with PGO beat Graal's own JIT on AWFY (0.892 vs 0.918 of C2, awfy_summary.txt). Oracle PetClinic: PGO native 11,902 vs C2 11,066 req/s. The synthesis lists both numbers but still says 'JIT owns peak in all'.
- Hermes JIT RSS (8.8-16.1 MB) equals Static Hermes native RSS (9.8-17 MB) in the same table, and bun (JIT) hello world is 10.5 MB and 5.77 ms. Startup and memory gaps are Node/V8 runtime effects, not AOT.
- Static Hermes's own optimization-notes.md lists missing ints, 5 inline slots, out-of-line array loads, redundant casts and little inlining. Its 1.5-3.2x gap is implementation debt, not a typed-AOT ceiling. scriptc on the same nbody ran at about C speed.
- oxc_transformer lowers [[Set]] class fields, parameter properties and enums into nodes with dummy SPAN (class.rs:442-471, enum.rs). The 99.94% span join was measured before the transform and does not cover the lowered code where class hints matter.
- .NET never instruments R2R code (dynpgo-tiers.md:97). The hybrid 'feedback keeps flowing' needs an instrumented re-tier or data ICs that double as feedback.
- GuardedDevirtualization.md's 15%/5% describes a weak, early, local JIT analysis. It is not a limit for whole-program or sound-type devirtualization.
- Total-deployment size favors AOT runtimes in the dual-mode VMs measured: Dart JIT VM 41.5 MB vs AOT runtime 4.8 MB, and ASP.NET NativeAOT -89%. This is relevant to the user's 'JIT for smaller size' premise.
- JSC (bun) beats V8 by 1.2-1.25x on richards and raytracer (results-table.md). V8 full is not a fixed ceiling, and 'faster than V8' is reachable through JIT engineering as well.
- Bundling and minification of production JS breaks span-keyed TS hints unless source maps are used. The ts-type-soundness report raised this as an open question, but the synthesis's risks do not include it.
